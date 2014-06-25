use std::io;
use std::io::{IoResult, IoError};
use std::io::{BufReader, MemWriter};
use std::str;
use std::fmt;
use std::from_str;
use std::ascii::StrAsciiExt;

use std::collections::hashmap::HashMap;

pub static DEFAULT_BLOCK_SIZE: uint = 512;

#[deriving(Show, Eq, PartialEq, Clone)]
pub enum Opcode {
    RRQ   = 0x01,
    WRQ   = 0x02,
    DATA  = 0x03,
    ACK   = 0x04,
    ERROR = 0x05,
    OACK  = 0x06
}

#[deriving(Eq, PartialEq, Clone)]
pub enum Mode {
    NetAscii,
    Octet
}

impl fmt::Show for Mode {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            NetAscii => write!(fmt, "netascii"),
            Octet => write!(fmt, "octet")
        }
    }
}

impl from_str::FromStr for Mode {
    fn from_str(s: &str) -> Option<Mode> {
        match s {
            "netascii" => Some(NetAscii),
            "octet" => Some(Octet),
            _ => None
        }
    }
}

#[deriving(Clone, Eq, PartialEq)]
pub enum RolloverMethod {
    Zero = 0u16,
    One  = 1u16
}

impl from_str::FromStr for RolloverMethod {
    fn from_str(s: &str) -> Option<RolloverMethod> {
        match s {
            "0" => Some(Zero),
            "1" => Some(One),
            _ => None
        }
    }
}

impl fmt::Show for RolloverMethod {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Zero => write!(fmt, "0"),
            One => write!(fmt, "1")
        }
    }
}


#[deriving(Show, Eq, PartialEq, Clone)]
pub enum Error {
    Undefined                 = 0x00,
    FileNotFound              = 0x01,
    AccessViolation           = 0x02,
    DiskFull                  = 0x03,
    IllegalOperation          = 0x04,
    UnknownTransferId         = 0x05,
    FileAlreadyExists         = 0x06,
    NoSuchUser                = 0x07,
    OptionNegotiationRejected = 0x08
}

impl Error {
    fn from_u16(code: u16) -> Option<Error> {
        match code {
            0 => Some(Undefined),
            1 => Some(FileNotFound),
            2 => Some(AccessViolation),
            3 => Some(DiskFull),
            4 => Some(IllegalOperation),
            5 => Some(UnknownTransferId),
            6 => Some(FileAlreadyExists),
            7 => Some(NoSuchUser),
            8 => Some(OptionNegotiationRejected),
            _ => None
        }
    }
}

pub type Filename = String;
pub type BlockId = u16;
pub type Options = HashMap<String, String>;

#[deriving(Show, Eq, PartialEq, Clone)]
pub enum Packet {
    ReadRequest(Filename, Mode, Options),
    WriteRequest(Filename, Mode, Options),
    Data(BlockId, Vec<u8>),
    Acknowledgment(BlockId),
    Error(Error, String),
    OptionAcknowledgment(Options)
}

impl Packet {
    pub fn opcode(&self) -> Opcode {
        match *self {
            ReadRequest(..) => RRQ,
            WriteRequest(..) => WRQ,
            Data(..) => DATA,
            Acknowledgment(..) => ACK,
            Error(..) => ERROR,
            OptionAcknowledgment(..) => OACK
        }
    }

    pub fn filename<'a>(&'a self) -> Option<&'a str> {
        match *self {
            ReadRequest(ref filename, _, _) | WriteRequest(ref filename, _, _) =>
                Some(filename.as_slice()),
            _ => None
        }
    }

    pub fn is_option_ack(&self) -> bool {
        match self {
            &OptionAcknowledgment(..) => true,
            _ => false
        }
    }

    pub fn to_ioerror(&self) -> Option<IoError> {
        match *self {
            Error(_, ref msg) => {
                Some(IoError {
                    kind: io::OtherIoError,
                    desc: "tftp protocol error",
                    detail: Some(msg.clone())
                })
            }
            _ => None
        }
    }

    pub fn encode(mode: Mode, p: &Packet) -> IoResult<Vec<u8>> {
        let mut w = MemWriter::new();
        try!(w.write_be_u16(p.opcode() as u16));
        match *p {
            ReadRequest(ref filename, mode, ref opts) | WriteRequest(ref filename, mode, ref opts) => {
                try!(w.write(filename.as_bytes()));
                try!(w.write_u8(0));
                try!(w.write(mode.to_str().as_bytes()));
                try!(w.write_u8(0));
                try!(Packet::encode_options(&mut w, opts));
            },
            Data(block_id, ref data) => {
                try!(w.write_be_u16(block_id));
                if mode == NetAscii {
                    try!(Packet::encode_netascii(&mut w, data.as_slice()));
                } else {
                    try!(w.write(data.as_slice()));
                }
            },
            Acknowledgment(block_id) => {
                try!(w.write_be_u16(block_id));
            },
            Error(err, ref msg) => {
                try!(w.write_be_u16(err as u16));
                try!(w.write(msg.as_bytes()));
                try!(w.write_u8(0));
            },
            OptionAcknowledgment(ref opts) => {
                try!(Packet::encode_options(&mut w, opts));
            }
        }
        Ok(Vec::from_slice(w.get_ref()))
    }

    fn encode_options(w: &mut MemWriter, opts: &Options) -> IoResult<()> {
        for key in opts.keys() {
            try!(w.write(key.as_bytes()));
            try!(w.write_u8(0));
            try!(w.write(opts.get(key).as_bytes()));
            try!(w.write_u8(0));
        }
        Ok(())
    }

    fn encode_netascii(w: &mut MemWriter, data: &[u8]) -> IoResult<()> {
        for b in data.iter() {
            if *b == '\n' as u8 {
                try!(w.write_str("\r\n"))
            } else if *b == '\r' as u8 {
                try!(w.write_str("\r\0"))
            } else {
                try!(w.write_u8(*b))
            }
        }
        return Ok(())
    }

    pub fn decode(mode: Mode, p: &[u8]) -> IoResult<Packet> {
        let mut buf = BufReader::new(p);
        let opcode = try!(buf.read_be_u16());
        if opcode == RRQ as u16 {
            Packet::decode_request(&mut buf, |fname, mode, opts| ReadRequest(fname, mode, opts))
        } else if opcode == WRQ as u16 {
            Packet::decode_request(&mut buf, |fname, mode, opts| WriteRequest(fname, mode, opts))
        } else if opcode == DATA as u16 {
            let block_id = try!(buf.read_be_u16());
            let data = try!(if mode == NetAscii {
                Packet::decode_netascii(&mut buf)
            } else {
                buf.read_to_end()
            });
            Ok(Data(block_id, data))
        } else if opcode == ACK as u16 {
            let block_id = try!(buf.read_be_u16());
            Ok(Acknowledgment(block_id))
        } else if opcode == ERROR as u16 {
            let error_code = try!(buf.read_be_u16());
            let error_msg = try!(Packet::read_str(&mut buf));
            match Error::from_u16(error_code) {
                Some(err) => Ok(Error(err, error_msg)),
                None => invalid_input_error("Invalid error code")
            }
        } else if opcode == OACK as u16 {
            let opts = Packet::decode_options(&mut buf);
            Ok(OptionAcknowledgment(opts))
        } else {
            invalid_input_error("Wrong packet type")
        }
    }

    fn decode_request(buf: &mut BufReader, f: |Filename, Mode, Options| -> Packet) -> IoResult<Packet> {
        let filename = try!(Packet::read_str(buf));
        let mode_name = try!(Packet::read_str(buf));
        let opts = Packet::decode_options(buf);
        match from_str::<Mode>(mode_name.as_slice()) {
            Some(mode) => Ok(f(filename, mode, opts)),
            None => invalid_input_error("Mode not recognized")
        }
    }

    fn read_to(buf: &mut BufReader, byte: u8) -> IoResult<Vec<u8>> {
        let mut res = Vec::new();

        let mut used;
        loop {
            {
                let available = match buf.fill_buf() {
                    Ok(n) => n,
                    Err(ref e) if res.len() > 0 && e.kind == io::EndOfFile => {
                        used = 0;
                        break
                    }
                    Err(e) => return Err(e)
                };
                match available.iter().position(|&b| b == byte) {
                    Some(i) => {
                        res.push_all(available.slice_to(i));
                        used = i + 1;
                        break
                    }
                    None => {
                        res.push_all(available);
                        used = available.len();
                    }
                }
            }
            buf.consume(used);
        }
        buf.consume(used);
        Ok(res)
    }

    fn read_str(buf: &mut BufReader) -> IoResult<String> {
        let bytes = try!(Packet::read_to(buf, 0));
        match str::from_utf8_owned(bytes.as_slice().to_owned()) {
            Ok(read_str) => Ok(read_str),
            Err(_) => invalid_input_error("Wrong string encoding")
        }
    }

    fn decode_options(buf: &mut BufReader) -> Options {
        let mut opts = HashMap::new();
        loop {
            let key_opt = Packet::read_str(buf);
            let val_opt = Packet::read_str(buf);
            match (key_opt, val_opt) {
                (Ok(key), Ok(val)) => { opts.insert(key.as_slice().to_ascii_lower(), val); },
                _ => break
            }
        }
        opts
    }

    fn decode_netascii(buf: &mut BufReader) -> IoResult<Vec<u8>> {
        let mut data = Vec::new();
        loop {
            match buf.read_byte() {
                Ok(b) => {
                    if b == '\r' as u8 {
                        let next = try!(buf.read_byte()) as char;
                        match next {
                            '\n' => data.push('\n' as u8),
                            '\0' => data.push('\r' as u8),
                            _    => return invalid_input_error("Invalid netascii encoding")
                        }
                    } else {
                        data.push(b);
                    }
                }
                Err(ref err) if err.kind == io::EndOfFile => break,
                Err(err) => return Err(err)
            }
        }
        return Ok(data)
    }
}

fn invalid_input_error<T>(desc: &'static str) -> IoResult<T> {
    let err = IoError {
        kind: io::InvalidInput,
        desc: desc,
        detail: None
    };
    Err(err)
}

#[cfg(test)]
mod test {
    use super::{Packet, Octet, NetAscii};
    use super::{ReadRequest, Data};

    #[test]
    fn option_names_are_parsed_case_insensitive() {
        let mut packet_bytes = Vec::from_slice([0u8, 1]);
        packet_bytes.push_all(b"file.ext\0octet\0Key\0Val\0");
        match Packet::decode(Octet, packet_bytes.as_slice()).unwrap() {
            ReadRequest(_, _, ref opts) => {
                assert_eq!(opts.get(&"key".to_string()), &"Val".to_string());
            },
            _ => fail!()
        }
    }

    #[test]
    fn encoding_and_decoding_data_in_octet_mode() {
        let data = b"CR\rNL\nEND\n";
        let packet = Data(9, Vec::from_slice(data));
        let mut packet_bytes = Vec::from_slice([0u8, 3, 0, 9]);
        packet_bytes.push_all(data);
        assert_eq!(Packet::encode(Octet, &packet).unwrap(), packet_bytes);
        assert_eq!(Packet::decode(Octet, packet_bytes.as_slice()).unwrap(), packet);
    }

    #[test]
    fn encoding_and_decoding_data_in_netascii_mode() {
        let packet = Data(1, Vec::from_slice(b"CR\rNL\nEND\n"));
        let mut packet_bytes = Vec::from_slice([0u8, 3, 0, 1]);
        packet_bytes.push_all(b"CR\r\0NL\r\nEND\r\n");
        assert_eq!(Packet::encode(NetAscii, &packet).unwrap(), packet_bytes);
        assert_eq!(Packet::decode(NetAscii, packet_bytes.as_slice()).unwrap(), packet);
    }
}

#[cfg(test)]
mod bench {
    extern crate test;

    use std::collections::hashmap::HashMap;
    use self::test::Bencher;

    use super::{Packet, Mode, Octet, NetAscii};
    use super::{ReadRequest, Data, Acknowledgment};

    fn bench_encode(b: &mut Bencher, p: &Packet, m: Mode) {
        let packet_bytes = Packet::encode(Octet, p).unwrap();
        b.iter(|| { Packet::encode(m, p) });
        b.bytes = packet_bytes.len() as u64;
    }

    fn bench_decode(b: &mut Bencher, p: &Packet, m: Mode) {
        let packet_bytes = Packet::encode(Octet, p).unwrap();
        b.iter(|| { Packet::decode(m, packet_bytes.as_slice()) });
        b.bytes = packet_bytes.len() as u64;
    }

    #[bench]
    fn encode_read_request(b: &mut Bencher) {
        bench_encode(b, &ReadRequest("file/name.ext".to_string(), Octet, HashMap::new()), Octet)
    }

    #[bench]
    fn decode_read_request(b: &mut Bencher) {
        bench_decode(b, &ReadRequest("file/name.ext".to_string(), Octet, HashMap::new()), Octet)
    }

    #[bench]
    fn encode_data_octet(b: &mut Bencher) {
        bench_encode(b, &Data(99, Vec::from_slice(b"hello\r\nworld\n")), Octet)
    }

    #[bench]
    fn decode_data_octet(b: &mut Bencher) {
        bench_decode(b, &Data(99, Vec::from_slice(b"hello\r\nworld\n")), Octet)
    }

    #[bench]
    fn encode_data_netascii(b: &mut Bencher) {
        bench_encode(b, &Data(99, Vec::from_slice(b"hello\r\nworld\n")), NetAscii)
    }

    #[bench]
    fn decode_data_netascii(b: &mut Bencher) {
        bench_decode(b, &Data(99, Vec::from_slice(b"hello\r\nworld\n")), NetAscii)
    }

    #[bench]
    fn encode_ack(b: &mut Bencher) {
        bench_encode(b, &Acknowledgment(21000), Octet)
    }

    #[bench]
    fn decode_ack(b: &mut Bencher) {
        bench_decode(b, &Acknowledgment(21000), Octet)
    }
}
