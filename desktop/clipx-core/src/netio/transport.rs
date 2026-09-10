use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use uuid::Uuid;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
};
use tokio_tungstenite::{
    WebSocketStream, accept_async, client_async, tungstenite::Message as WsMessage,
};

use crate::message::proto::{self, peer_message::Body};

#[derive(Debug, Clone)]
pub struct ConnectedDevice {
    pub id: String,
    pub connection_id: String,
    pub addr: SocketAddr,
}

#[derive(Debug)]
pub enum TransportEvent {
    Connected(ConnectedDevice),
    Disconnected { device_id: String, connection_id: String },
    ConnectFailed(String),
    /// A decoded PeerMessage from an identified device. Pre-transport
    /// variants are consumed by DeviceManager; Body::Clipboard is defined
    /// but not routed anywhere yet.
    PeerMessage {
        device_id: String,
        connection_id: String,
        message: proto::PeerMessage,
    },
}

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
    DisconnectOnConnection {
        device_id: String,
        connection_id: String,
        reply_to: oneshot::Sender<bool>,
    },
    PromoteConnection {
        device_id: String,
        connection_id: String,
        reply_to: oneshot::Sender<bool>,
    },
    SendPeerMessage {
        device_id: String,
        message: proto::PeerMessage,
        reply_to: oneshot::Sender<bool>,
    },
    SendPeerMessageOnConnection {
        device_id: String,
        connection_id: String,
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
        connection_id: String,
        addr: SocketAddr,
        outbound_tx: mpsc::Sender<WsMessage>,
        reply_to: Option<oneshot::Sender<bool>>,
        allow_duplicate: bool,
    },
    Failed {
        device_id: String,
        reply_to: Option<oneshot::Sender<bool>>,
    },
    Closed {
        device_id: String,
        connection_id: String,
    },
}

struct Conn {
    connection_id: String,
    addr: SocketAddr,
    outbound_tx: mpsc::Sender<WsMessage>,
}

pub struct Transport {
    listener: TcpListener,
    connections: HashMap<String, Conn>,
    // A duplicate inbound ConnectChallenge gets a temporary connection so the
    // device manager can see the competing request and explicitly resolve it.
    // These are never used for clipboard/file traffic until promoted.
    pending_connections: HashMap<String, (String, Conn)>,
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
            pending_connections: HashMap::new(),
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
                self.pending_connections.retain(|_, (pending_device_id, _)| pending_device_id != &device_id);
                let _ = reply_to.send(removed);
            }
            TransportCommand::DisconnectOnConnection {
                device_id,
                connection_id,
                reply_to,
            } => {
                let remove_active = self
                    .connections
                    .get(&device_id)
                    .is_some_and(|conn| conn.connection_id == connection_id);
                let remove_pending = self
                    .pending_connections
                    .get(&connection_id)
                    .is_some_and(|(pending_device_id, conn)| pending_device_id == &device_id && conn.connection_id == connection_id);
                let _ = if remove_active {
                    self.connections.remove(&device_id);
                    let _ = self.event_tx.send(TransportEvent::Disconnected {
                        device_id: device_id.clone(),
                        connection_id: connection_id.clone(),
                    });
                    reply_to.send(true)
                } else if remove_pending {
                    self.pending_connections.remove(&connection_id);
                    reply_to.send(true)
                } else {
                    reply_to.send(false)
                };
            }
            TransportCommand::PromoteConnection {
                device_id,
                connection_id,
                reply_to,
            } => {
                let Some((pending_device_id, pending_conn)) = self.pending_connections.remove(&connection_id) else {
                    let _ = reply_to.send(false);
                    return;
                };
                if pending_device_id != device_id {
                    self.pending_connections.insert(connection_id, (pending_device_id, pending_conn));
                    let _ = reply_to.send(false);
                    return;
                }

                // The manager explicitly chose this competing connection for
                // the handshake. Drop the former active connection without
                // emitting another disconnect event; the manager is already
                // switching its logical session to this connection.
                self.connections.remove(&device_id);
                self.connections.insert(device_id, pending_conn);
                let _ = reply_to.send(true);
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
                let Some(outbound_tx) = self.connections.get(&device_id).map(|c| c.outbound_tx.clone()) else {
                    let _ = reply_to.send(false);
                    return;
                };
                tokio::spawn(async move {
                    let ok = outbound_tx.send(WsMessage::binary(buf)).await.is_ok();
                    let _ = reply_to.send(ok);
                });
            }
            TransportCommand::SendPeerMessageOnConnection { device_id, connection_id, message, reply_to } => {
                let mut buf = Vec::new();
                if message.encode(&mut buf).is_err() {
                    let _ = reply_to.send(false);
                    return;
                }
                let outbound_tx = if let Some(conn) = self.connections.get(&device_id) {
                    if conn.connection_id == connection_id {
                        Some(conn.outbound_tx.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }.or_else(|| {
                    self.pending_connections
                        .get(&connection_id)
                        .filter(|(pending_device_id, conn)| pending_device_id == &device_id && conn.connection_id == connection_id)
                        .map(|(_, conn)| conn.outbound_tx.clone())
                });
                let Some(outbound_tx) = outbound_tx else {
                    let _ = reply_to.send(false);
                    return;
                };
                tokio::spawn(async move {
                    let ok = outbound_tx.send(WsMessage::binary(buf)).await.is_ok();
                    let _ = reply_to.send(ok);
                });
            }
            TransportCommand::ListConnections { reply_to } => {
                let devices = self
                    .connections
                    .iter()
                    .map(|(id, c)| ConnectedDevice {
                        id: id.clone(),
                        connection_id: c.connection_id.clone(),
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
                connection_id,
                addr,
                outbound_tx,
                reply_to,
                allow_duplicate,
            } => {
                if self.connections.contains_key(&device_id) {
                    if allow_duplicate {
                        self.pending_connections.insert(
                            connection_id.clone(),
                            (device_id.clone(), Conn { connection_id: connection_id.clone(), addr, outbound_tx }),
                        );
                        if let Some(r) = reply_to { let _ = r.send(true); }
                        tracing::debug!(%device_id, %connection_id, "duplicate inbound connect connection retained for handshake arbitration");
                    } else {
                        drop(outbound_tx);
                        if let Some(r) = reply_to { let _ = r.send(false); }
                        tracing::debug!(%device_id, %connection_id, "duplicate transport connection rejected");
                    }
                    return;
                }
                self.connections.insert(device_id.clone(), Conn { connection_id: connection_id.clone(), addr, outbound_tx });
                if let Some(r) = reply_to { let _ = r.send(true); }
                let _ = self.event_tx.send(TransportEvent::Connected(ConnectedDevice { id: device_id, connection_id, addr }));
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
            Registration::Closed { device_id, connection_id } => {
                if self.connections.get(&device_id).is_some_and(|c| c.connection_id == connection_id) {
                    self.connections.remove(&device_id);
                    let _ = self.event_tx.send(TransportEvent::Disconnected { device_id, connection_id });
                } else if self.pending_connections.remove(&connection_id).is_some() {
                    tracing::debug!(%device_id, %connection_id, "pending handshake connection closed");
                }
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
            let connection_id = Uuid::new_v4().to_string();
            let (outbound_tx, outbound_rx) = mpsc::channel(8);
            let _ = register_tx.send(Registration::Ok {
                device_id: device_id.clone(),
                connection_id: connection_id.clone(),
                addr,
                outbound_tx,
                reply_to: Some(reply_to),
                allow_duplicate: false,
            });
            run_connection(ws, device_id, connection_id, outbound_rx, event_tx, register_tx.clone()).await;
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
            let allow_duplicate = matches!(message.body, Some(Body::ConnectChallenge(_)));
            let device_id = match &message.body {
                Some(Body::PairRequest(req)) => hex::encode(&req.fingerprint),
                Some(Body::ConnectChallenge(c)) => hex::encode(&c.initiator_fingerprint),
                _ => {
                    tracing::warn!("inbound connection from {addr} sent unexpected first message");
                    return;
                }
            };

            let connection_id = Uuid::new_v4().to_string();
            let (outbound_tx, outbound_rx) = mpsc::channel(8);
            let (accepted_tx, accepted_rx) = oneshot::channel();
            if register_tx.send(Registration::Ok {
                device_id: device_id.clone(),
                connection_id: connection_id.clone(),
                addr,
                outbound_tx,
                reply_to: Some(accepted_tx),
                allow_duplicate,
            }).is_err() {
                return;
            }

            // Do not feed the first handshake frame to DeviceManager unless
            // transport actually accepted this connection. Otherwise a
            // rejected duplicate could still mutate the connection session.
            if !accepted_rx.await.unwrap_or(false) {
                return;
            }

            let _ = event_tx.send(TransportEvent::PeerMessage {
                device_id: device_id.clone(),
                connection_id: connection_id.clone(),
                message,
            });

            run_connection(ws, device_id, connection_id, outbound_rx, event_tx, register_tx.clone()).await;
        });
    }
}

/// Drives one connection for its lifetime. No split — select! holds `&mut ws`
/// across both its send and receive branches directly.
async fn run_connection(
    mut ws: WebSocketStream<TcpStream>,
    device_id: String,
    connection_id: String,
    mut outbound_rx: mpsc::Receiver<WsMessage>,
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
                            Ok(message) => { let _ = event_tx.send(TransportEvent::PeerMessage { device_id: device_id.clone(), connection_id: connection_id.clone(), message }); }
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
        connection_id: connection_id.clone(),
    });
}
