use std::io;
use std::u64;
use std::io::{IoResult, IoError};
use std::io::Timer;
use std::io::net::ip::SocketAddr;
use std::comm::Select;
use std::hash::Hash;
use std::from_str;
use std::default::Default;

use collections::hashmap::HashMap;

use protocol::DEFAULT_BLOCK_SIZE;
use protocol::{Mode, RolloverMethod, Options, Octet};
use protocol::{Packet, Error, UnknownTransferId};

#[deriving(Show, Clone)]
pub struct TransferOptions {
    mode: Mode,
    block_size: uint,
    transfer_size: Option<u64>,
    receive_timeout: u64,
    resend_timeout: u64,
    rollover: Option<RolloverMethod>
}

fn find_as<K: Hash + TotalEq, T: from_str::FromStr>(h: &HashMap<K, ~str>, key: K) -> Option<T> {
    h.find(&key).and_then(|s| from_str::<T>(*s))
}

impl TransferOptions {
    pub fn to_options(&self) -> Options {
        let mut h = HashMap::new();
        let defaults: TransferOptions = Default::default();
        self.insert_to(&mut h, ~"blksize", &defaults, |o| o.block_size);
        self.insert_to(&mut h, ~"timeout", &defaults, |o| o.resend_timeout);
        self.insert_to_opt(&mut h, ~"tsize", &defaults, |o| o.transfer_size);
        self.insert_to_opt(&mut h, ~"rollover", &defaults, |o| o.rollover);
        h
    }

    fn insert_to<T: ToStr + Eq>(&self, h: &mut Options, key: ~str, defaults: &TransferOptions, f: |&TransferOptions| -> T) {
        if f(self) != f(defaults) {
            h.insert(key, f(self).to_str());
        }
    }

    fn insert_to_opt<T: ToStr + Eq>(&self, h: &mut Options, key: ~str, defaults: &TransferOptions, f: |&TransferOptions| -> Option<T>) {
        if f(self) != f(defaults) {
             h.insert(key, f(self).unwrap().to_str());
        }
    }

    pub fn from_map(opts: &Options) -> TransferOptions {
        let mut default: TransferOptions = Default::default();
        for key in opts.keys() {
            match key.as_slice() {
                "blksize" => {
                    default.block_size = find_as(opts, ~"blksize").unwrap_or(default.block_size);
                }
                "tsize" => {
                    default.transfer_size = find_as(opts, ~"tsize");
                }
                "timeout" => {
                    default.resend_timeout = find_as(opts, ~"timeout").unwrap_or(default.resend_timeout);
                }
                "rollover" => {
                    default.rollover = find_as(opts, ~"rollover");
                }
                _ => continue
            }
        }
        default
    }
}

impl Default for TransferOptions {
    fn default() -> TransferOptions {
        TransferOptions {
            mode: Octet,
            block_size: DEFAULT_BLOCK_SIZE,
            transfer_size: None,
            receive_timeout: 5000,
            resend_timeout: 1000,
            rollover: None
        }
    }
}

pub struct LoopData<T, D> {
    remote_addr: SocketAddr,
    reader_port: Receiver<(SocketAddr, Packet)>,
    writer_chan: Sender<(SocketAddr, Packet)>,
    opts: TransferOptions,
    current_id: u16,
    resend: bool,
    path_handle: T,
    data: D
}

#[deriving(Eq, Show)]
enum Selected {
    Timeout,
    ResendTimeout,
    ReceivePacket
}

pub enum LoopControl<T> {
    Normal,
    Break,
    Continue,
    Return(T)
}

pub struct Void;

macro_rules! control( ($e:expr) => {
    match $e {
        Normal => {},
        Break => break,
        Continue => continue,
        Return(v) => return v
    }
})

pub fn receive_loop<T, D>(mut d: LoopData<T, D>,
                          resend: bool,
                          init: |&LoopData<T, D>|,
                          loop_start: |&mut LoopData<T, D>| -> LoopControl<IoResult<()>>,
                          handle_packet: |&mut LoopData<T, D>, bool, &Packet, &mut bool| -> LoopControl<IoResult<()>>) -> IoResult<()> {

    let mut timer = try!(Timer::new());
    let mut resend_timer = try!(Timer::new());
    let mut first = true;

    let mut timeout = timer.oneshot(d.opts.receive_timeout);
    let mut reset_timeout = false;

    init(&d);
    loop {
        let mut resend_timeout = if resend {
            resend_timer.oneshot(d.opts.resend_timeout)
        } else {
            resend_timer.oneshot(u64::MAX)
        };
        control!(loop_start(&mut d));
        if reset_timeout {
            timeout = timer.oneshot(d.opts.receive_timeout);
            reset_timeout = false;
        }
        let selected = {
            let select = Select::new();
            let mut timeout_handle = select.handle(&mut timeout);
            let mut resend_timeout_handle = select.handle(&mut resend_timeout);
            let mut reader_handle = select.handle(&mut d.reader_port);
            unsafe {
                timeout_handle.add();
                resend_timeout_handle.add();
                reader_handle.add();
            }
            let select_id = select.wait();
            if select_id == timeout_handle.id() {
                info!("Connection timeout");
                Timeout
            } else if select_id == resend_timeout_handle.id() {
                info!("Resend timeout");
                d.resend = true;
                ResendTimeout
            } else {
                ReceivePacket
            }
        };
        if selected == Timeout {
            return Err(IoError {
                kind: io::ConnectionAborted,
                desc: "Connection timeout",
                detail: None
            })
        } else if selected == ResendTimeout {
            continue
        }
        let (addr, packet) = d.reader_port.recv();
        if addr != d.remote_addr && !first {
            warn!("Different TID: {}, {}", addr.to_str(), d.remote_addr.to_str());
            let err_packet = Error(UnknownTransferId, ~"Unknown TID");
            d.writer_chan.send((addr, err_packet))
        } else {
            let first_packet = first;
            if first {
                if addr.ip == d.remote_addr.ip {
                    first = false;
                    d.remote_addr = addr;
                } else {
                    continue
                }
            }
            match packet {
                err@Error(..) => return Err(err.to_ioerror().unwrap()),
                _ => {}
            }
            if first_packet && !packet.is_option_ack() {
                d.opts = Default::default();
            }
            control!(handle_packet(&mut d, first_packet, &packet, &mut reset_timeout));
        }
    }
    Ok(())
}
