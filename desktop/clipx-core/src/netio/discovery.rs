use crate::device::identity::DeviceIdentity;
use crate::message::proto;
use prost::Message;
use socket2::{Domain, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, watch},
    task::JoinHandle,
    time::Duration,
};

/// Sets up discovery's socket and spawns both the broadcaster and listener
/// tasks. Also returns the shared socket and pending-requests map, since
/// DeviceManager needs both to send its own challenge requests on the same
/// socket and correlate their responses.
pub fn spawn(
    shutdown_rx: watch::Receiver<bool>,
    identity: Arc<DeviceIdentity>,
    discovered_tx: mpsc::UnboundedSender<(proto::Announce, SocketAddr)>,
) -> std::io::Result<Vec<JoinHandle<()>>> {
    let socket = Arc::new(make_shared_socket()?);

    let broadcast_task = {
        let socket = socket.clone();
        let shutdown_rx = shutdown_rx.clone();
        let identity = identity.clone();
        tokio::spawn(async move { broadcast_presence(shutdown_rx, identity, socket).await })
    };

    let listen_task = {
        let socket = socket.clone();
        tokio::spawn(async move {
            listen_for_devices(shutdown_rx, identity, discovered_tx, socket).await
        })
    };

    Ok(vec![broadcast_task, listen_task])
}

/// SO_REUSEADDR (+ SO_REUSEPORT on unix) so a second local process — another
/// clipx-core instance, or the tests/emulator tool — can bind this same port
/// on the same machine instead of failing outright. Self-discovery is still
/// safe: the listener below filters by identity fingerprint, not by IP/port,
/// so a second local instance is just seen as another (real) peer.
fn make_shared_socket() -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    let _ = socket.set_reuse_port(true);
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&"0.0.0.0:9999".parse::<SocketAddr>().unwrap().into())?;
    UdpSocket::from_std(socket.into())
}

async fn broadcast_presence(
    mut shutdown_rx: watch::Receiver<bool>,
    identity: Arc<DeviceIdentity>,
    socket: Arc<UdpSocket>,
) {
    let device_type = crate::device::config::get_current_device_type() as i32;
    let announce = proto::Announce {
        fingerprint: identity.get_this_device_fingerprint().to_vec(),
        device_name: crate::device::config::get_hostname(),
        device_type,
        ws_port: crate::device::config::get_ws_port(),
    };

    let mut message: Vec<u8> = vec![];
    announce.encode(&mut message).unwrap();

    tracing::info!("Upd broadcasting to 255.255.255.255:9999 after each 2 sec");
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                let _ = socket.send_to(&message, "255.255.255.255:9999").await;
            }
        }
    }
    tracing::info!("broadcaster stopped");
}

async fn listen_for_devices(
    mut shutdown_rx: watch::Receiver<bool>,
    identity: Arc<DeviceIdentity>,
    discovered_tx: mpsc::UnboundedSender<(proto::Announce, SocketAddr)>,
    socket: Arc<UdpSocket>,
) {
    let mut buf = [0u8; 1024];

    tracing::info!("Upd listening to peer broadcasts/announces");
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
            result = socket.recv_from(&mut buf) => {
                let Ok((len, src_addr)) = result else { continue; };

                let announce = match proto::Announce::decode(&buf[..len]) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("failed to decode envelope from {src_addr}: {e}");
                        continue;
                    }
                };

                if announce.fingerprint == identity.get_this_device_fingerprint() {
                    continue;
                }
                let _ = discovered_tx.send((announce, src_addr));
            }
        }
    }
    tracing::info!("listener stopped");
}
