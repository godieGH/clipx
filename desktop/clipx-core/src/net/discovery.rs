use crate::device::identity::DeviceIdentity;
use crate::message::proto;
use crate::net::pending_requests::PendingRequests;
use prost::Message;
use std::net::SocketAddr;
use std::net::UdpSocket as StdUdpSocket;
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
    device_id: String,
    identity: Arc<DeviceIdentity>,
    discovered_tx: mpsc::UnboundedSender<(proto::Announce, SocketAddr)>,
) -> std::io::Result<(
    Vec<JoinHandle<()>>,
    Arc<UdpSocket>,
    PendingRequests<proto::ChallengeResponse>,
)> {
    let socket = Arc::new(make_shared_socket()?);
    let pending = PendingRequests::new();

    let broadcast_task = {
        let socket = socket.clone();
        let shutdown_rx = shutdown_rx.clone();
        let identity = identity.clone();
        let device_id = device_id.clone();
        tokio::spawn(async move {
            broadcast_presence(shutdown_rx, device_id, identity, socket).await
        })
    };

    let listen_task = {
        let socket = socket.clone();
        let pending = pending.clone();
        tokio::spawn(async move {
            listen_for_devices(shutdown_rx, device_id, identity, discovered_tx, pending, socket)
                .await
        })
    };

    Ok((vec![broadcast_task, listen_task], socket, pending))
}

fn make_shared_socket() -> std::io::Result<UdpSocket> {
    let std_socket = StdUdpSocket::bind("0.0.0.0:9999")?;
    std_socket.set_broadcast(true)?;
    std_socket.set_nonblocking(true)?;
    UdpSocket::from_std(std_socket)
}

async fn broadcast_presence(
    mut shutdown_rx: watch::Receiver<bool>,
    device_id: String,
    identity: Arc<DeviceIdentity>,
    socket: Arc<UdpSocket>,
) {
    let announce = proto::Announce {
        device_id,
        device_name: crate::device::config::get_hostname(),
        device_type: proto::DeviceType::Windows as i32,
        ws_port: 8080,
        public_key: identity.public_key_bytes().to_vec(),
    };

    let envelope = proto::UdpEnvelope {
        payload: Some(proto::udp_envelope::Payload::Announce(announce)),
    };
    let mut message: Vec<u8> = vec![];
    envelope.encode(&mut message).unwrap();

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

async fn listen_for_devices(
    mut shutdown_rx: watch::Receiver<bool>,
    device_id: String,
    identity: Arc<DeviceIdentity>,
    discovered_tx: mpsc::UnboundedSender<(proto::Announce, SocketAddr)>,
    pending: PendingRequests<proto::ChallengeResponse>,
    socket: Arc<UdpSocket>,
) {
    let mut buf = [0u8; 1024];

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
            result = socket.recv_from(&mut buf) => {
                let Ok((len, src_addr)) = result else { continue; };

                let envelope = match proto::UdpEnvelope::decode(&buf[..len]) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("failed to decode envelope from {src_addr}: {e}");
                        continue;
                    }
                };

                match envelope.payload {
                    Some(proto::udp_envelope::Payload::Announce(announce)) => {
                        if announce.device_id == device_id { continue; }
                        let _ = discovered_tx.send((announce, src_addr));
                    }
                    Some(proto::udp_envelope::Payload::Challenge(challenge)) => {
                        let signature = identity.sign(&challenge.nonce);
                        let response = proto::UdpEnvelope {
                            payload: Some(proto::udp_envelope::Payload::ChallengeResponse(
                                proto::ChallengeResponse {
                                    request_id: challenge.request_id,
                                    signature: signature.to_vec(),
                                },
                            )),
                        };
                        let mut out = Vec::new();
                        response.encode(&mut out).unwrap();
                        let _ = socket.send_to(&out, src_addr).await;
                    }
                    Some(proto::udp_envelope::Payload::ChallengeResponse(resp)) => {
                        pending.resolve(&resp.request_id, resp.clone()).await;
                    }
                    None => {}
                }
            }
        }
    }
    tracing::info!("listener stopped");
}