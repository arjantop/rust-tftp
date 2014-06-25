extern crate green;
extern crate rustuv;
extern crate tftp;

use std::io;
use std::io::fs::{File};
use std::io::BufferedWriter;
use std::io::net::ip::{SocketAddr, Ipv4Addr};
use std::default::Default;

use tftp::client;

#[start]
fn start(argc: int, argv: **u8) -> int {
    green::start(argc, argv, rustuv::event_loop, main)
}

fn main() {
    let args = std::os::args();
    let path = Path::new("/tmp/tftp_test");
    let opts: tftp::TransferOptions = Default::default();
    let mut file = BufferedWriter::new(File::open_mode(&path, io::Truncate, io::Write));
    let result = client::get(SocketAddr {
        ip: Ipv4Addr(127, 0, 0, 1),
        port: 69
    }, Path::new(args[1]), opts, &mut file);
    println!("Result: {}", result);
}
