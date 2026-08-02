use crate::message::proto;
use prost::Message;
use std::{net::UdpSocket as StdUdpSocket};
use tokio::{net::UdpSocket, sync::{watch, mpsc}, time::Duration};
use std::net::SocketAddr;

fn make_broadcast_socket() -> std::io::Result<UdpSocket> {
    let std_socket = StdUdpSocket::bind("0.0.0.0:0")?;
    std_socket.set_broadcast(true)?;
    std_socket.set_nonblocking(true)?; // required before handing to Tokio
    UdpSocket::from_std(std_socket)
}

pub async fn broadcast_presence(mut shutdown_rx: watch::Receiver<bool>, device_id: String) {
    let socket = make_broadcast_socket().expect("failed to create broadcast socket");
    let mut message: Vec<u8> = vec![];
    let announce = proto::Announce {
        device_id,
        device_name: crate::device::config::get_hostname(),
        device_type: proto::DeviceType::Windows as i32,
        ws_port: 8080, // what is this or it for websocket later
    };

    announce.encode(&mut message).unwrap();

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                let _ = socket.send_to(&message, "255.255.255.255:9999").await;
                tracing::info!("broadcasting...");
            }
        }
    }
    tracing::info!("broadcaster stopped");
}

pub async fn listen_for_devices(
    mut shutdown_rx: watch::Receiver<bool>,
    device_id: String,
    discovered_tx: mpsc::UnboundedSender<(proto::Announce, SocketAddr)>
) {
    let socket = UdpSocket::bind("0.0.0.0:9999").await.expect("bind failed");
    let mut buf = [0u8; 1024];

    loop {
        tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
                result = socket.recv_from(&mut buf) => {
                    if let Ok((len, src_addr)) = result {
                        let announce = match proto::Announce::decode(&buf[..len]) {
                            Ok(a) => a,
                            Err(e) => {
                                tracing::warn!("failed to decode announce from {src_addr}: {e}");
                                continue;
                            }
                        };

                        if announce.device_id == device_id {
                            continue;
                        }
                        tracing::info!("Descover device IP = {src_addr} {:?}", announce);
                        let _ = discovered_tx.send((announce, src_addr));
                    }
            }
        }
    }
    tracing::info!("listener stopped");
}
