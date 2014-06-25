use std::io;
use std::io::IoResult;
use std::io::net::ip::{SocketAddr, Ipv4Addr};

use protocol::{ReadRequest, WriteRequest, Data, Acknowledgment};
use protocol::{OptionAcknowledgment, Packet, One};
use util::{socket_reader, socket_writer, bind_socket};

use common::TransferOptions;
use common::{receive_loop, LoopData, Void, Normal, Break, Return};

pub fn get(remote_addr: SocketAddr, path: Path, opts: TransferOptions, w: &mut Writer) -> IoResult<()> {
    let socket = try!(bind_socket(Ipv4Addr(127, 0, 0, 1)));
    let reader_recv = socket_reader(socket.clone(), opts.mode, opts.block_size + 4);
    let writer_snd = socket_writer(socket, opts.mode);

    get_internal(reader_recv, writer_snd, remote_addr, path, opts, w)
}

fn get_internal(reader_recv: Receiver<(SocketAddr, Packet)>,
                writer_snd: Sender<(SocketAddr, Packet)>,
                remote_addr: SocketAddr,
                path: Path,
                opts: TransferOptions,
                w: &mut Writer) -> IoResult<()> {

    let loop_data = LoopData {
        remote_addr: remote_addr,
        reader_port: reader_recv,
        writer_chan: writer_snd,
        opts: opts,
        current_id: 1,
        resend: true,
        path_handle: w,
        data: Void
    };
    receive_loop(loop_data, false, |d| {
        let path_str = path.as_str().unwrap().into_string();
        d.writer_chan.send((remote_addr, ReadRequest(path_str, d.opts.mode, d.opts.to_options())));
    }, |_| Normal, |d, first_packet, packet, reset| {
        match *packet {
            OptionAcknowledgment(ref topts) if first_packet => {
                d.opts = TransferOptions::from_map(topts);
                d.writer_chan.send((d.remote_addr, Acknowledgment(0)));
            }
            Data(block_id, ref data) if block_id == d.current_id => {
                if d.current_id == ::std::u16::MAX && d.opts.rollover == Some(One) {
                    d.current_id = d.opts.rollover.map(|r| r as u16).unwrap_or(0);
                } else {
                    d.current_id += 1;
                }
                *reset = true;
                match d.path_handle.write(data.as_slice()) {
                    Ok(_) => {}
                    err@Err(_) => return Return(err)
                }
                d.writer_chan.send((d.remote_addr, Acknowledgment(block_id)));
                if data.len() < d.opts.block_size {
                    return Break
                }
            }
            _ => {}
        }
        Normal
    })
}

pub fn read_block(r: &mut Reader, block_size: uint) -> IoResult<Vec<u8>> {
    let mut buf = Vec::from_elem(block_size, 0u8);
    match r.read(buf.as_mut_slice()) {
        Ok(len) => {
            if len == block_size {
                Ok(buf)
            } else {
                Ok(Vec::from_slice(buf.slice_to(len)))
            }
        }
        Err(err) => {
            if err.kind == io::EndOfFile {
                Ok(Vec::new())
            } else {
                Err(err)
            }
        }
    }
}

pub fn put(remote_addr: SocketAddr, path: Path, opts: TransferOptions, r: &mut Reader) -> IoResult<()> {
    let socket = try!(bind_socket(Ipv4Addr(127, 0, 0, 1)));
    let reader_recv = socket_reader(socket.clone(), opts.mode, opts.block_size + 4);
    let writer_snd = socket_writer(socket, opts.mode);

    put_internal(reader_recv, writer_snd, remote_addr, path, opts, r)
}

fn put_internal(reader_recv: Receiver<(SocketAddr, Packet)>,
                writer_snd: Sender<(SocketAddr, Packet)>,
                remote_addr: SocketAddr,
                path: Path,
                opts: TransferOptions,
                r: &mut Reader) -> IoResult<()> {

    let loop_data = LoopData {
        remote_addr: remote_addr,
        reader_port: reader_recv,
        writer_chan: writer_snd,
        opts: opts,
        current_id: 0,
        resend: false,
        path_handle: r,
        data: None
    };
    receive_loop(loop_data, true, |d| {
        let path_str = path.as_str().unwrap().into_string();
        d.writer_chan.send((d.remote_addr, WriteRequest(path_str, d.opts.mode, d.opts.to_options())));
    }, |d| {
        if d.resend {
            if d.data.is_none() {
                match read_block(d.path_handle, d.opts.block_size) {
                    Ok(data) => d.data = Some(data),
                    Err(err) => return Return(Err(err))
                }
            }
            let data = Vec::from_slice(d.data.as_ref().unwrap().as_slice());
            d.writer_chan.send((d.remote_addr, Data(d.current_id, data)));
            d.resend = false;
        }
        Normal
    }, |d, first_packet, packet, reset| {
        match *packet {
            OptionAcknowledgment(ref topts) if first_packet=> {
                d.opts = TransferOptions::from_map(topts);
                d.current_id += 1;
                d.resend = true;
            }
            Acknowledgment(block_id) if block_id == d.current_id => {
                if d.data.is_some() && d.data.as_ref().unwrap().len() < d.opts.block_size {
                     return Break
                }
                if d.current_id == ::std::u16::MAX && d.opts.rollover == Some(One) {
                    d.current_id = d.opts.rollover.map(|r| r as u16).unwrap_or(0);
                } else {
                    d.current_id += 1;
                }
                *reset = true;
                d.resend = true;
                d.data = None;
            }
            _ => ()
        }
        Normal
    })
}

#[cfg(test)]
mod test {
    use std::io;
    use std::io::{IoResult, IoError};
    use std::io::net::ip::{SocketAddr, Ipv4Addr};
    use std::default::Default;

    use std::collections::HashMap;

    use super::{get_internal, put_internal};
    use common::TransferOptions;
    use protocol::DEFAULT_BLOCK_SIZE;
    use protocol::{Packet, Data, Acknowledgment, ReadRequest, Octet, WriteRequest, Zero, One, OptionAcknowledgment};

    static LOCALHOST: SocketAddr = SocketAddr {
        ip: Ipv4Addr(127, 0, 0, 1),
        port: 60000
    };

    static ERR_TIMEOUT: IoError = IoError {
        kind: io::ConnectionAborted,
        desc: "Connection timeout",
        detail: None
    };

    fn gen_data(len: uint) -> Vec<u8> {
        gen_data_sized(512, len)
    }

    fn gen_data_sized(block_size: u16, len: uint) -> Vec<u8> {
        Vec::from_fn(len, |i| ((i / block_size as uint) % 256) as u8)
    }

    fn receive_all(recv: &Receiver<(SocketAddr, Packet)>) -> Vec<Packet> {
        recv.iter().map(|(_addr, p)| p).collect()
    }

    fn get_assert_received_opts(opts: TransferOptions, data: &[u8], received: &[Packet], expected: &[Packet]) -> IoResult<()> {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");
        let mut writer = io::MemWriter::new();
        for packet in received.iter() {
            reader_snd.send((LOCALHOST, packet.clone()));
        }
        let res = get_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut writer);
        println!("result = {}", res);
        let sent = receive_all(&writer_rcv);
        assert_eq!(expected, sent.as_slice());
        assert_eq!(data, writer.get_ref());
        res
    }
    fn get_assert_received(data: &[u8], received: &[Packet], expected: &[Packet]) -> IoResult<()> {
        let mut opts: TransferOptions = Default::default();
        opts.receive_timeout = 2;
        get_assert_received_opts(opts, data, received, expected)
    }

    #[test]
    fn get_receives_one_packet_sized_data() {
        let data = gen_data(511);
        assert_eq!(get_assert_received(data.as_slice(),
                                       [Data(1, Vec::from_elem(511, 0u8))],
                                       [ReadRequest("/path".to_string(), Octet, HashMap::new()),
                                        Acknowledgment(1)]), Ok(()));
    }

    #[test]
    fn get_receives_packet_of_max_packet_size() {
        let data = gen_data(DEFAULT_BLOCK_SIZE);
        assert_eq!(get_assert_received(data.as_slice(),
                                       [Data(1, Vec::from_elem(512, 0u8)),
                                        Data(2, Vec::from_elem(0, 1u8))],
                                       [ReadRequest("/path".to_string(), Octet, HashMap::new()),
                                        Acknowledgment(1),
                                        Acknowledgment(2)]), Ok(()));
    }

    #[test]
    fn get_receives_multi_packet_data() {
        let data = gen_data(DEFAULT_BLOCK_SIZE*2 + 10);
        assert_eq!(get_assert_received(data.as_slice(),
                                       [Data(1, Vec::from_elem(512, 0u8)),
                                        Data(2, Vec::from_elem(512, 1u8)),
                                        Data(3, Vec::from_elem(10, 2u8))],
                                       [ReadRequest("/path".to_string(), Octet, HashMap::new()),
                                        Acknowledgment(1),
                                        Acknowledgment(2),
                                        Acknowledgment(3)]), Ok(()));
    }

    #[test]
    fn get_timeouts_if_not_receiving_packets() {
        let res = get_assert_received([], [], [ReadRequest("/path".to_string(), Octet, HashMap::new())]);
        assert_eq!(Err(ERR_TIMEOUT.clone()), res);
    }

    #[test]
    fn get_error_on_writing_to_writer() {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, _writer_rcv) = channel();
        let path = Path::new("/path");
        let mut opts: TransferOptions = Default::default();
        opts.receive_timeout = 2;
        let mut buf = [0u8, ..100];
        let mut writer = io::BufWriter::new(buf);
        for i in range(1, 3) {
            let d = Vec::from_elem(DEFAULT_BLOCK_SIZE, i as u8);
            reader_snd.send((LOCALHOST, Data(i as u16, d)));
        }
        let res = get_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut writer);
        assert!(res.is_err());
    }

    #[test]
    fn get_ignores_unexpected_packets() {
        let data = gen_data(DEFAULT_BLOCK_SIZE*2 + 90);
        assert_eq!(get_assert_received(data.as_slice(),
                                       [Data(1, Vec::from_elem(512, 0u8)),
                                        Acknowledgment(0),
                                        Data(2, Vec::from_elem(512, 1u8)),
                                        Data(1, Vec::from_elem(512, 0u8)),
                                        Data(3, Vec::from_elem(90, 2u8))],
                                       [ReadRequest("/path".to_string(), Octet, HashMap::new()),
                                        Acknowledgment(1),
                                        Acknowledgment(2),
                                        Acknowledgment(3)]), Ok(()));
    }

    #[test]
    fn get_does_rollover_to_zero() {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");

        static MAX: uint = ::std::u16::MAX as uint;
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), 1.to_str());

        let mut writer = io::MemWriter::new();
        reader_snd.send((LOCALHOST, OptionAcknowledgment(topts.clone())));
        for i in range(1, MAX + 1) {
            reader_snd.send((LOCALHOST, Data(i as u16, Vec::from_slice([0u8]))));
        }
        reader_snd.send((LOCALHOST, Data(0, Vec::from_slice([0u8]))));
        reader_snd.send((LOCALHOST, Data(1, Vec::from_slice([]))));

        let mut expected = Vec::from_slice([ReadRequest("/path".to_string(), Octet, topts)]);
        for i in range(0, MAX + 1) {
            expected.push(Acknowledgment(i as u16));
        }
        expected.push(Acknowledgment(0 as u16));
        expected.push(Acknowledgment(1 as u16));

        let res = get_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut writer);
        println!("result = {}", res);
        let sent = receive_all(&writer_rcv);
        for (e, s) in expected.iter().zip(sent.iter()) {
            assert_eq!(e, s);
        }
        assert!(writer.get_ref().len() == MAX + 1);
        assert_eq!(Ok(()), res);
    }

    #[test]
    fn get_does_rollover_to_one() {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");

        static MAX: uint = ::std::u16::MAX as uint;
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1;
        opts.rollover = Some(One);

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), 1.to_str());
        topts.insert("rollover".to_string(), 1.to_str());

        let mut writer = io::MemWriter::new();
        reader_snd.send((LOCALHOST, OptionAcknowledgment(topts.clone())));
        for i in range(1, MAX + 1) {
            reader_snd.send((LOCALHOST, Data(i as u16, Vec::from_slice([0u8]))));
        }
        reader_snd.send((LOCALHOST, Data(1, Vec::from_slice([0u8]))));
        reader_snd.send((LOCALHOST, Data(2, Vec::from_slice([]))));

        let mut expected = Vec::from_slice([ReadRequest("/path".to_string(), Octet, topts)]);
        for i in range(0, MAX + 1) {
            expected.push(Acknowledgment(i as u16));
        }
        expected.push(Acknowledgment(1 as u16));
        expected.push(Acknowledgment(2 as u16));

        let res = get_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut writer);
        println!("result = {}", res);
        let sent = receive_all(&writer_rcv);
        for (e, s) in expected.iter().zip(sent.iter()) {
            assert_eq!(e, s);
        }
        assert!(writer.get_ref().len() == MAX + 1);
        assert_eq!(Ok(()), res);
    }

    #[test]
    fn get_non_default_options_are_sent_in_request() {
        let data = gen_data(0);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;
        opts.transfer_size = Some(0);
        opts.receive_timeout = 20;
        opts.resend_timeout = 11;
        opts.rollover = Some(Zero);

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());
        topts.insert("tsize".to_string(), "0".to_string());
        topts.insert("timeout".to_string(), "11".to_string());
        topts.insert("rollover".to_string(), "0".to_string());
        assert_eq!(get_assert_received_opts(opts, data.as_slice(),
                                            [Data(1, Vec::new())],
                                            [ReadRequest("/path".to_string(), Octet, topts),
                                             Acknowledgment(1)]), Ok(()));
    }

    #[test]
    fn get_not_acknowledged_options_are_not_used() {
        let data = gen_data(DEFAULT_BLOCK_SIZE + 2);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());
        assert_eq!(get_assert_received_opts(opts, data.as_slice(),
                                            [Data(1, Vec::from_elem(DEFAULT_BLOCK_SIZE, 0u8)),
                                             Data(2, Vec::from_elem(2, 1u8))],
                                            [ReadRequest("/path".to_string(), Octet, topts),
                                             Acknowledgment(1),
                                             Acknowledgment(2)]), Ok(()));
    }

    #[test]
    fn get_only_acknowledged_options_are_used() {
        let data = gen_data_sized(256, 256 + 9);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());

        let mut topts_ack = HashMap::new();
        topts_ack.insert("blksize".to_string(), "256".to_string());
        assert_eq!(get_assert_received_opts(opts, data.as_slice(),
                                            [OptionAcknowledgment(topts_ack),
                                             Data(1, Vec::from_elem(256, 0u8)),
                                             Data(2, Vec::from_elem(9, 1u8))],
                                            [ReadRequest("/path".to_string(), Octet, topts),
                                             Acknowledgment(0),
                                             Acknowledgment(1),
                                             Acknowledgment(2)]), Ok(()));
    }

    #[test]
    fn get_options_are_only_accepted_when_they_are_first_received_packet() {
        let data = gen_data(300);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 400;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "400".to_string());

        let mut topts2 = HashMap::new();
        topts2.insert("blksize".to_string(), "256".to_string());
        assert_eq!(get_assert_received_opts(opts, data.as_slice(),
                                            [OptionAcknowledgment(topts.clone()),
                                             OptionAcknowledgment(topts2),
                                             Data(1, Vec::from_elem(300, 0u8))],
                                            [ReadRequest("/path".to_string(), Octet, topts),
                                             Acknowledgment(0),
                                             Acknowledgment(1)]), Ok(()));
    }

    fn put_assert_sent_opts(opts: TransferOptions, reader: &mut Reader, received: &[Packet], expected: &[Packet]) -> IoResult<()> {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");
        for packet in received.iter() {
            reader_snd.send((LOCALHOST, packet.clone()));
        }
        let res = put_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, reader);
        let sent = receive_all(&writer_rcv);
        println!("result = {}", res);
        assert_eq!(expected, sent.as_slice());
        res
    }

    fn put_assert_sent_buf(reader: &mut Reader, received: &[Packet], expected: &[Packet]) -> IoResult<()> {
        let mut opts: TransferOptions = Default::default();
        opts.receive_timeout = 10;
        put_assert_sent_opts(opts, reader, received, expected)
    }

    fn put_assert_sent(data: &[u8], received: &[Packet], expected: &[Packet]) -> IoResult<()> {
        let mut reader = io::BufReader::new(data);
        put_assert_sent_buf(&mut reader, received, expected)
    }

    #[test]
    fn put_sends_one_packet_sized_data() {
        let data = gen_data(111);
        assert_eq!(put_assert_sent(data.as_slice(),
                                   [Acknowledgment(0),
                                    Acknowledgment(1)],
                                   [WriteRequest("/path".to_string(), Octet, HashMap::new()),
                                    Data(1, Vec::from_elem(111, 0u8))]), Ok(()));
    }

    #[test]
    fn put_sends_one_packet_data_of_max_packet_size() {
        let data = gen_data(DEFAULT_BLOCK_SIZE);
        assert_eq!(put_assert_sent(data.as_slice(),
                                   [Acknowledgment(0),
                                    Acknowledgment(1),
                                    Acknowledgment(2)],
                                   [WriteRequest("/path".to_string(), Octet, HashMap::new()),
                                    Data(1, Vec::from_elem(DEFAULT_BLOCK_SIZE, 0u8)),
                                    Data(2, Vec::from_elem(0, 1u8))]), Ok(()));
    }

    #[test]
    fn put_sends_multi_packet_sized_data() {
        let data = gen_data(DEFAULT_BLOCK_SIZE + 200);
        assert_eq!(put_assert_sent(data.as_slice(),
                                   [Acknowledgment(0),
                                    Acknowledgment(1),
                                    Acknowledgment(2)],
                                   [WriteRequest("/path".to_string(), Octet, HashMap::new()),
                                    Data(1, Vec::from_elem(512, 0u8)),
                                    Data(2, Vec::from_elem(200, 1u8))]), Ok(()));
    }

    #[test]
    fn put_timeouts_if_not_receiving_packets() {
        let res = put_assert_sent([], [], [WriteRequest("/path".to_string(), Octet, HashMap::new())]);
        assert_eq!(Err(ERR_TIMEOUT.clone()), res);
    }

    #[test]
    fn put_returns_error_on_reader_error() {
        let data = [];
        let mut reader = io::BufReader::new(data);
        let res = put_assert_sent_buf(&mut reader, [],
                                      [WriteRequest("/path".to_string(), Octet, HashMap::new())]);
        assert!(res.is_err());
    }

    #[test]
    fn put_resends_data_on_no_received_ack() {
        let mut opts: TransferOptions = Default::default();
        opts.receive_timeout = 5;
        opts.resend_timeout = 3;
        let data = gen_data(DEFAULT_BLOCK_SIZE + 11);
        let mut reader = io::BufReader::new(data.as_slice());
        let mut topt = HashMap::new();
        topt.insert("timeout".to_string(), 3.to_str());
        let res = put_assert_sent_opts(opts, &mut reader, [OptionAcknowledgment(topt.clone())],
                                       [WriteRequest("/path".to_string(), Octet, topt),
                                        Data(1, Vec::from_elem(512, 0u8)),
                                        Data(1, Vec::from_elem(512, 0u8))]);
        assert_eq!(Err(ERR_TIMEOUT.clone()), res);
    }

    #[test]
    fn put_ignores_unexpected_packages() {
        let data = gen_data(DEFAULT_BLOCK_SIZE + 10);
        assert_eq!(put_assert_sent(data.as_slice(),
                                   [Data(1, Vec::new()),
                                    Acknowledgment(0),
                                    Acknowledgment(2),
                                    Acknowledgment(1),
                                    Acknowledgment(2)],
                                   [WriteRequest("/path".to_string(), Octet, HashMap::new()),
                                    Data(1, Vec::from_elem(512, 0u8)),
                                    Data(2, Vec::from_elem(10, 1u8))]), Ok(()));
    }

    #[test]
    fn put_does_rollover_to_zero() {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");

        static MAX: uint = ::std::u16::MAX as uint;
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1;
        let data = Vec::from_elem(MAX + 1, 0u8);
        let mut reader = io::BufReader::new(data.as_slice());
        let mut topt = HashMap::new();
        topt.insert("blksize".to_string(), 1.to_str());

        reader_snd.send((LOCALHOST, OptionAcknowledgment(topt.clone())));
        for i in range(1, MAX + 1) {
            reader_snd.send((LOCALHOST, Acknowledgment(i as u16)));
        }
        reader_snd.send((LOCALHOST, Acknowledgment(0)));
        reader_snd.send((LOCALHOST, Acknowledgment(1)));

        let mut expected = Vec::from_slice([WriteRequest("/path".to_string(), Octet, topt)]);
        for i in range(1, MAX + 1) {
            expected.push(Data(i as u16, Vec::from_slice([0u8])));
        }
        expected.push(Data(0, Vec::from_slice([0u8])));
        expected.push(Data(1, Vec::new()));

        let res = put_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut reader);
        println!("result = {}", res);
        let sent = receive_all(&writer_rcv);
        for (e, s) in expected.iter().zip(sent.iter()) {
            assert_eq!(e, s);
        }
        assert_eq!(Ok(()), res);
   }

    #[test]
    fn put_does_rollover_to_one() {
        let (reader_snd, reader_rcv) = channel();
        let (writer_snd, writer_rcv) = channel();
        let path = Path::new("/path");

        static MAX: uint = ::std::u16::MAX as uint;
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1;
        opts.rollover = Some(One);
        let data = Vec::from_elem(MAX + 1, 0u8);
        let mut reader = io::BufReader::new(data.as_slice());
        let mut topt = HashMap::new();
        topt.insert("blksize".to_string(), 1.to_str());
        topt.insert("rollover".to_string(), 1.to_str());

        reader_snd.send((LOCALHOST, OptionAcknowledgment(topt.clone())));
        for i in range(1, MAX + 1) {
            reader_snd.send((LOCALHOST, Acknowledgment(i as u16)));
        }
        reader_snd.send((LOCALHOST, Acknowledgment(1)));
        reader_snd.send((LOCALHOST, Acknowledgment(2)));

        let mut expected = Vec::from_slice([WriteRequest("/path".to_string(), Octet, topt)]);
        for i in range(1, MAX + 1) {
            expected.push(Data(i as u16, Vec::from_slice([0u8])));
        }
        expected.push(Data(1, Vec::from_slice([0u8])));
        expected.push(Data(2, Vec::new()));

        let res = put_internal(reader_rcv, writer_snd, LOCALHOST, path, opts, &mut reader);
        println!("result = {}", res);
        let sent = receive_all(&writer_rcv);
        for (e, s) in expected.iter().zip(sent.iter()) {
            assert_eq!(e, s);
        }
        assert_eq!(Ok(()), res);
    }

    #[test]
    fn put_non_default_options_are_sent_in_request() {
        let data = gen_data(0);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;
        opts.transfer_size = Some(0);
        opts.receive_timeout = 20;
        opts.resend_timeout = 11;
        opts.rollover = Some(Zero);

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());
        topts.insert("tsize".to_string(), "0".to_string());
        topts.insert("timeout".to_string(), "11".to_string());
        topts.insert("rollover".to_string(), "0".to_string());

        let mut reader = io::BufReader::new(data.as_slice());
        assert_eq!(put_assert_sent_opts(opts, &mut reader,
                                            [Acknowledgment(0),
                                             Acknowledgment(1)],
                                            [WriteRequest("/path".to_string(), Octet, topts),
                                             Data(1, Vec::new())]), Ok(()));
    }

    #[test]
    fn put_not_acknowledged_options_are_not_used() {
        let data = gen_data(DEFAULT_BLOCK_SIZE + 2);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());
        let mut reader = io::BufReader::new(data.as_slice());
        assert_eq!(put_assert_sent_opts(opts, &mut reader,
                                            [Acknowledgment(0),
                                             Acknowledgment(1),
                                             Acknowledgment(2)],
                                            [WriteRequest("/path".to_string(), Octet, topts),
                                             Data(1, Vec::from_elem(DEFAULT_BLOCK_SIZE, 0u8)),
                                             Data(2, Vec::from_elem(2, 1u8))]), Ok(()));
    }

    #[test]
    fn put_only_acknowledged_options_are_used() {
        let data = gen_data_sized(256, 256 + 9);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 1024;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "1024".to_string());

        let mut topts_ack = HashMap::new();
        topts_ack.insert("blksize".to_string(), "256".to_string());
        let mut reader = io::BufReader::new(data.as_slice());
        assert_eq!(put_assert_sent_opts(opts, &mut reader,
                                            [OptionAcknowledgment(topts_ack),
                                             Acknowledgment(1),
                                             Acknowledgment(2)],
                                            [WriteRequest("/path".to_string(), Octet, topts),
                                             Data(1, Vec::from_elem(256, 0u8)),
                                             Data(2, Vec::from_elem(9, 1u8))]), Ok(()));
    }

    #[test]
    fn put_options_are_only_accepted_when_they_are_first_received_packet() {
        let data = gen_data(300);
        let mut opts: TransferOptions = Default::default();
        opts.block_size = 400;

        let mut topts = HashMap::new();
        topts.insert("blksize".to_string(), "400".to_string());

        let mut topts2 = HashMap::new();
        topts2.insert("blksize".to_string(), "256".to_string());
        let mut reader = io::BufReader::new(data.as_slice());
        assert_eq!(put_assert_sent_opts(opts, &mut reader,
                                            [OptionAcknowledgment(topts.clone()),
                                             OptionAcknowledgment(topts2),
                                             Acknowledgment(1)],
                                            [WriteRequest("/path".to_string(), Octet, topts),
                                             Data(1, Vec::from_elem(300, 0u8))]), Ok(()));
    }
}
