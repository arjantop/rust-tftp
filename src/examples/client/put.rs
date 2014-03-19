extern crate tftp;

use std::io::fs::{File};
use std::io::BufferedReader;
use std::io::net::ip::{SocketAddr, Ipv4Addr};
use std::default::Default;

use tftp::client;

fn main() {
    let args = std::os::args();
    let opts: tftp::TransferOptions = Default::default();
    let path = Path::new(args[2].clone());
    let mut file = BufferedReader::new(File::open(&path));
    let result = client::put(SocketAddr {
        ip: Ipv4Addr(127, 0, 0, 1),
        port: 69
    }, Path::new(args[1]), opts, &mut file);
    println!("Result: {}", result);
}
