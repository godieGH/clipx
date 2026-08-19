use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
};
use tokio_tungstenite::{
    WebSocketStream, accept_async, client_async, tungstenite::Message as WsMessage,
};

use crate::message::proto::{self, peer_message::Body};

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct ConnectedDevice {
    pub id: String,
    pub addr: SocketAddr,
}

#[allow(unused)]
#[derive(Debug)]
pub enum TransportEvent {
    Connected(ConnectedDevice),
    Disconnected(String),
    ConnectFailed(String),
    /// A decoded PeerMessage from an identified device. Pre-transport
    /// variants are consumed by DeviceManager; Body::Clipboard is defined
    /// but not routed anywhere yet.
    PeerMessage {
        device_id: String,
        message: proto::PeerMessage,
    },
}

#[allow(unused)]
#[derive(Debug)]
pub enum TransportCommand {
    Connect {
        device_id: String,
        addr: SocketAddr,
        reply_to: oneshot::Sender<bool>,
    },
    Disconnect {
        device_id: String,
        reply_to: oneshot::Sender<bool>,
    },
    SendPeerMessage {
        device_id: String,
        message: proto::PeerMessage,
        reply_to: oneshot::Sender<bool>,
    },
    ListConnections {
        reply_to: oneshot::Sender<Vec<ConnectedDevice>>,
    },
}

enum Registration {
    Ok {
        device_id: String,
        addr: SocketAddr,
        outbound_tx: mpsc::UnboundedSender<WsMessage>,
        reply_to: Option<oneshot::Sender<bool>>,
    },
    Failed {
        device_id: String,
        reply_to: Option<oneshot::Sender<bool>>,
    },
    Closed {
        device_id: String,
    },
}

struct Conn {
    addr: SocketAddr,
    outbound_tx: mpsc::UnboundedSender<WsMessage>,
}

pub struct Transport {
    listener: TcpListener,
    connections: HashMap<String, Conn>,
    shutdown_rx: watch::Receiver<bool>,
    command_rx: mpsc::UnboundedReceiver<TransportCommand>,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    register_tx: mpsc::UnboundedSender<Registration>,
    register_rx: mpsc::UnboundedReceiver<Registration>,
}

impl Transport {
    pub async fn create_transport(
        shutdown_rx: watch::Receiver<bool>,
        command_rx: mpsc::UnboundedReceiver<TransportCommand>,
        event_tx: mpsc::UnboundedSender<TransportEvent>,
    ) -> Self {
        let ws_port = crate::device::config::get_ws_port();
        let listener = TcpListener::bind(format!("0.0.0.0:{ws_port}"))
            .await
            .expect("websocket listener failed bind");
        tracing::info!("WS server running on port={ws_port} ...");
        let (register_tx, register_rx) = mpsc::unbounded_channel();
        Self {
            listener,
            connections: HashMap::new(),
            shutdown_rx,
            command_rx,
            event_tx,
            register_tx,
            register_rx,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() { break; }
                }
                Some(cmd) = self.command_rx.recv() => {
                    self.handle_command(cmd);
                }
                Some(reg) = self.register_rx.recv() => {
                    self.handle_registration(reg);
                }
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, addr)) => self.spawn_inbound(stream, addr),
                        Err(e) => tracing::warn!("transport accept error: {e}"),
                    }
                }
            }
        }
        tracing::info!("transport stopped");
    }

    fn handle_command(&mut self, cmd: TransportCommand) {
        match cmd {
            TransportCommand::Connect {
                device_id,
                addr,
                reply_to,
            } => {
                if self.connections.contains_key(&device_id) {
                    tracing::warn!("dial requested for {device_id} which is already registered; ignoring (state caller)");
                    let _ = reply_to.send(true);
                    return;
                }
                self.spawn_outbound(device_id, addr, reply_to);
            }
            TransportCommand::Disconnect {
                device_id,
                reply_to,
            } => {
                // Dropping outbound_tx is what tells the connection's task
                // to stop and close the socket — see run_connection.
                let removed = self.connections.remove(&device_id).is_some();
                let _ = reply_to.send(removed);
            }
            TransportCommand::SendPeerMessage {
                device_id,
                message,
                reply_to,
            } => {
                let mut buf = Vec::new();
                if message.encode(&mut buf).is_err() {
                    let _ = reply_to.send(false);
                    return;
                }
                let ok = self
                    .connections
                    .get(&device_id)
                    .map(|c| c.outbound_tx.send(WsMessage::binary(buf)).is_ok())
                    .unwrap_or(false);
                let _ = reply_to.send(ok);
            }
            TransportCommand::ListConnections { reply_to } => {
                let devices = self
                    .connections
                    .iter()
                    .map(|(id, c)| ConnectedDevice {
                        id: id.clone(),
                        addr: c.addr,
                    })
                    .collect();
                let _ = reply_to.send(devices);
            }
        }
    }

    fn handle_registration(&mut self, reg: Registration) {
        match reg {
            Registration::Ok {
                device_id,
                addr,
                outbound_tx,
                reply_to,
            } => {
                self.connections
                    .insert(device_id.clone(), Conn { addr, outbound_tx });
                if let Some(r) = reply_to {
                    let _ = r.send(true);
                }
                let _ = self
                    .event_tx
                    .send(TransportEvent::Connected(ConnectedDevice {
                        id: device_id,
                        addr,
                    }));
            }
            Registration::Failed {
                device_id,
                reply_to,
            } => {
                if let Some(r) = reply_to {
                    let _ = r.send(false);
                }
                let _ = self.event_tx.send(TransportEvent::ConnectFailed(device_id));
            }
            Registration::Closed { device_id } => {
                self.connections.remove(&device_id);
            }
        }
    }

    fn spawn_outbound(&self, device_id: String, addr: SocketAddr, reply_to: oneshot::Sender<bool>) {
        let register_tx = self.register_tx.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let stream = match tokio::time::timeout(
                Duration::from_secs(8),
                TcpStream::connect(addr),
            )
            .await
            {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::warn!("connect to {device_id} at {addr} failed: {e}");
                    let _ = register_tx.send(Registration::Failed {
                        device_id,
                        reply_to: Some(reply_to),
                    });
                    return;
                }
                Err(_) => {
                    tracing::warn!("connect to {device_id} at {addr} timed out");
                    let _ = register_tx.send(Registration::Failed {
                        device_id,
                        reply_to: Some(reply_to),
                    });
                    return;
                }
            };
            let ws = match client_async(format!("ws://{addr}/"), stream).await {
                Ok((ws, _resp)) => ws,
                Err(e) => {
                    tracing::warn!("ws handshake with {device_id} failed: {e}");
                    let _ = register_tx.send(Registration::Failed {
                        device_id,
                        reply_to: Some(reply_to),
                    });
                    return;
                }
            };
            let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
            let _ = register_tx.send(Registration::Ok {
                device_id: device_id.clone(),
                addr,
                outbound_tx,
                reply_to: Some(reply_to),
            });
            run_connection(ws, device_id, outbound_rx, event_tx, register_tx.clone()).await;
        });
    }

    fn spawn_inbound(&self, stream: TcpStream, addr: SocketAddr) {
        let register_tx = self.register_tx.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let mut ws = match accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    tracing::warn!("inbound ws handshake failed from {addr}: {e}");
                    return;
                }
            };

            // First frame must self-identify — a fresh PairRequest (carries
            // a public key + fingerprint) or a ConnectChallenge (carries the
            // initiator's already-trusted fingerprint). Anything else on a
            // brand-new connection is a protocol violation.
            let first_bytes = match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
                Ok(Some(Ok(WsMessage::Binary(bytes)))) => bytes,
                _ => {
                    tracing::warn!(
                        "inbound connection from {addr} did not identify itself in time"
                    );
                    return;
                }
            };
            let message = match proto::PeerMessage::decode(first_bytes.as_ref()) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        "inbound connection from {addr} sent an undecodable first frame: {e}"
                    );
                    return;
                }
            };
            let device_id = match &message.body {
                Some(Body::PairRequest(req)) => hex::encode(&req.fingerprint),
                Some(Body::ConnectChallenge(c)) => hex::encode(&c.initiator_fingerprint),
                _ => {
                    tracing::warn!("inbound connection from {addr} sent unexpected first message");
                    return;
                }
            };

            let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
            let _ = register_tx.send(Registration::Ok {
                device_id: device_id.clone(),
                addr,
                outbound_tx,
                reply_to: None,
            });
            let _ = event_tx.send(TransportEvent::PeerMessage {
                device_id: device_id.clone(),
                message,
            });

            run_connection(ws, device_id, outbound_rx, event_tx, register_tx.clone()).await;
        });
    }
}

/// Drives one connection for its lifetime. No split — select! holds `&mut ws`
/// across both its send and receive branches directly.
async fn run_connection(
    mut ws: WebSocketStream<TcpStream>,
    device_id: String,
    mut outbound_rx: mpsc::UnboundedReceiver<WsMessage>,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    register_tx: mpsc::UnboundedSender<Registration>,
) {
    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if ws.send(WsMessage::Ping(Vec::new().into())).await.is_err() { break; }
            }
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(msg) => { if ws.send(msg).await.is_err() { break; } }
                    None => break, // Disconnect dropped the sender
                }
            }
            incoming = ws.next() => {
                match incoming {
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        match proto::PeerMessage::decode(bytes.as_ref()) {
                            Ok(message) => { let _ = event_tx.send(TransportEvent::PeerMessage { device_id: device_id.clone(), message }); }
                            Err(e) => tracing::warn!("undecodable frame from {device_id}: {e}"),
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => { tracing::warn!("read error from {device_id}: {e}"); break; }
                }
            }
        }
    }
    let _ = ws.close(None).await;
    let _ = register_tx.send(Registration::Closed {
        device_id: device_id.clone(),
    });
    let _ = event_tx.send(TransportEvent::Disconnected(device_id));
}
