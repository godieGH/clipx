use std::net::UdpSocket;

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:9999")?;
    let mut buf = [0u8; 1024];

    loop {
        let (len, src_addr) = socket.recv_from(&mut buf)?;
        let msg = String::from_utf8_lossy(&buf[..len]);
        println!("Heard from {src_addr}: {msg}");
    }

}