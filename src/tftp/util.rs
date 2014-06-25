use std::io::{IoResult, IoError, InvalidInput};
use std::io::net::udp::UdpSocket;
use std::io::net::ip::{SocketAddr, IpAddr};

use std::rand::random;

use protocol::{Mode, Packet};

pub fn random_ephemeral_port() -> u16 {
    let min = 49152;
    let max = 65535;
    random::<u16>() % (max - min) + min
}

pub fn receive_packet(socket: &mut UdpSocket, mode: Mode, buf: &mut [u8]) -> IoResult<(SocketAddr, Packet)> {
    let (len, addr) = try!(socket.recvfrom(buf));
    debug!("[{}] Got {} bytes: {}", addr.to_str(), len, buf.slice_to(len).to_str());
    let packet_bytes = buf.slice_to(len);
    match Packet::decode(mode, packet_bytes) {
        Ok(packet) => {
            info!("[{}] Got packet {}", addr.to_str(), packet.to_str());
            Ok((addr, packet))
        },
        Err(err) => {
            warn!("[{}] Error decoding packet: {}", addr.to_str(), err);
            debug!("[{}] Packet bytes: {}", addr.to_str(), packet_bytes.to_str());
            Err(err)
        }
    }
}

pub fn send_packet(socket: &mut UdpSocket, addr: &SocketAddr, mode: Mode, p: &Packet) -> IoResult<()> {
    match Packet::encode(mode, p) {
        Ok(packet_bytes) => {
            try!(socket.sendto(packet_bytes.as_slice(), *addr));
            info!("[{}] Sent packet: {}", addr.to_str(), p.to_str());
            Ok(())
        },
        Err(err) => {
            error!("[{}] Encoding packet failed with '{}': {}", addr.to_str(), err, p.to_str());
            Err(IoError {
                kind: InvalidInput,
                desc: "Error encoding packet",
                detail: None
            })
        }
    }

}

pub fn bind_socket(addr: IpAddr) -> IoResult<UdpSocket> {
    let rand_port = random_ephemeral_port();
    UdpSocket::bind(SocketAddr {
        ip: addr,
        port: rand_port
    })
}

pub fn socket_reader(us: UdpSocket, mode: Mode, packet_size: uint) -> Receiver<(SocketAddr, Packet)> {
    let (snd, rcv) = channel();
    spawn(proc() {
        let mut socket = us;
        let mut buf = Vec::from_elem(packet_size, 0u8);
        loop {
            match receive_packet(&mut socket, mode, buf.as_mut_slice()) {
                Ok(res) => snd.send(res),
                Err(err) => warn!("Error occured while reading: {}", err)
            }
        }
    });
    rcv
}

pub fn socket_writer(us: UdpSocket, mode: Mode) -> Sender<(SocketAddr, Packet)> {
    let (snd, rcv) = channel::<(SocketAddr, Packet)>();
    spawn(proc() {
        let mut socket = us;
        loop {
            match rcv.recv_opt() {
                Ok((addr, packet)) => {
                    let res = send_packet(&mut socket, &addr, mode, &packet);
                    if res.is_err() {
                        info!("Error occured while writing: {}", res.unwrap_err())
                    }
                },
                Err(_) => {
                    info!("Closing writer");
                    return
                }
            }
        }
    });
    snd
}

