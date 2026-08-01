use std::net::UdpSocket;
use std::time::{Duration};

fn main() -> std::io::Result<()> {

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;

    let message = b"Clipx-Hello id=abc123 name=Godie-Pc";

    loop {
        socket.send_to(message, "255.255.255.255:9999")?;
        println!("sent announcement");
        std::thread::sleep(Duration::from_secs(2));
    }
}