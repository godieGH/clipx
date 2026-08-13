use std::{collections::HashMap, net::SocketAddr};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct ConnectedDevice {
    pub id: String,
    pub addr: SocketAddr,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub enum TransportEvent {
    Connected(ConnectedDevice),
    Disconnected(String),
}

#[allow(unused)]
#[derive(Debug)]
pub enum TransportCommand {
    Connect { device_id: String, reply_to: oneshot::Sender<bool> },
    Disconnect { device_id: String, reply_to: oneshot::Sender<bool> },
    SendClipboard { device_id: String, payload: String, reply_to: oneshot::Sender<bool> },
    ListConnections { reply_to: oneshot::Sender<Vec<ConnectedDevice>> },
}

pub struct Transport {
    listener: TcpListener,
    connections: HashMap<String, ConnectedDevice>,
    shutdown_rx: watch::Receiver<bool>,
    command_rx: mpsc::UnboundedReceiver<TransportCommand>,
}

impl Transport {
    pub async fn create_transport(
        shutdown_rx: watch::Receiver<bool>,
        command_rx: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> Self {
        let ws_port = crate::device::config::get_ws_port();
        let listener = TcpListener::bind(format!("0.0.0.0:{ws_port}"))
            .await
            .expect("websocket listener failed bind");
        tracing::info!("WS server running on port={ws_port} ...");
        Self {
            listener,
            connections: HashMap::new(),
            shutdown_rx,
            command_rx,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        break;
                    }
                }
                Some(cmd) = self.command_rx.recv() => {
                    self.handle_command(cmd);
                }
                connection = self.listener.accept() => {
                    match connection {
                        Ok((socket, addr)) => {
                            tracing::info!("transport accepted connection from {addr}");
                            let _ = socket;
                        }
                        Err(err) => tracing::warn!("transport accept error: {err}"),
                    }
                }
            }
        }

        tracing::info!("transport stopped");
    }

    fn handle_command(&mut self, cmd: TransportCommand) {
        match cmd {
            TransportCommand::Connect { device_id, reply_to } => {
                let connected = self.connections.contains_key(&device_id);
                if !connected {
                    self.connections.insert(
                        device_id.clone(),
                        ConnectedDevice { id: device_id, addr: "0.0.0.0:0".parse().unwrap() },
                    );
                }
                let _ = reply_to.send(true);
            }
            TransportCommand::Disconnect { device_id, reply_to } => {
                let removed = self.connections.remove(&device_id).is_some();
                let _ = reply_to.send(removed);
            }
            TransportCommand::SendClipboard { device_id, payload, reply_to } => {
                let connected = self.connections.contains_key(&device_id);
                if connected {
                    tracing::info!("clipboard payload queued for {device_id}: {payload}");
                }
                let _ = reply_to.send(connected);
            }
            TransportCommand::ListConnections { reply_to } => {
                let devices = self.connections.values().cloned().collect();
                let _ = reply_to.send(devices);
            }
        }
    }
}

