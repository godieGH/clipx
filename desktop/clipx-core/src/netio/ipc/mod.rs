use crate::{
    clipboard::manager::ClipboardCommand,
    device::manager::{DeviceCommands, SeenMode},
    message::proto::clipx,
};
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, ToNsName,
    tokio::{Listener, Stream as LocalStream},
    traits::tokio::Listener as ListenerTrait,
};
use prost::Message;
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message as WsMessage};
use tokio_util::task::TaskTracker;

pub struct IpcService {
    name: String,
    listener: Option<Listener>,
    core_service_shutdown_rx: watch::Receiver<bool>,
    device_tx: UnboundedSender<DeviceCommands>,
    clipboard_tx: UnboundedSender<ClipboardCommand>,
    tracker: TaskTracker,
}

impl IpcService {
    pub fn new(
        name: impl Into<String>,
        shutdown_rx: watch::Receiver<bool>,
        device_tx: UnboundedSender<DeviceCommands>,
        clipboard_tx: UnboundedSender<ClipboardCommand>,
    ) -> Self {
        Self {
            name: name.into(),
            listener: None,
            core_service_shutdown_rx: shutdown_rx,
            device_tx,
            clipboard_tx,
            tracker: TaskTracker::new(),
        }
    }

    /// Same shape as before — spawn/await this like any other core task.
    /// No caller-side changes needed.
    pub async fn start(&mut self) {
        let name = self.create_name().unwrap();
        let listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        self.listener = Some(listener);

        tracing::info!("IPC services started");

        loop {
            tokio::select! {
                            _ = self.core_service_shutdown_rx.changed() => {
                                if *self.core_service_shutdown_rx.borrow() {
                                    tracing::info!("IPC service stopping — closing client connections");
                                    break;
                                }
                            }
                            conn = self.listener.as_ref().unwrap().accept() => {
                match conn {
                    Ok(stream) => {
                        let device_tx = self.device_tx.clone();
                        let clipboard_tx = self.clipboard_tx.clone();
                        let client_shutdown_rx = self.core_service_shutdown_rx.clone();
                        self.tracker.spawn(async move {
                            if let Err(err) =
                                IpcService::handle_client(stream, device_tx, clipboard_tx, client_shutdown_rx).await
                            {
                                tracing::error!(error = %err, "IPC client handler failed");
                            }
                        });
                    }
                    Err(e) => tracing::error!("Accept error: {e:?}"),
                }
            }
                        }
        }

        // Stop accepting new tasks into the tracker, then wait for every
        // in-flight client task to actually finish (each is racing the same
        // shutdown signal internally, so this resolves promptly).
        self.tracker.close();
        self.tracker.wait().await;
        tracing::info!("All IPC client connections closed");
    }

    async fn handle_client(
        conn: LocalStream,
        device_tx: UnboundedSender<DeviceCommands>,
        clipboard_tx: UnboundedSender<ClipboardCommand>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        let mut ws = accept_async(conn).await?;

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        let _ = ws.close(None).await;
                        break;
                    }
                }
                msg = ws.next() => {
                    let Some(msg) = msg else { break };
                    let msg = msg?;
                    match msg {
                        WsMessage::Close(_) => break,
                        WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
                        WsMessage::Binary(bytes) => {
                            let ipcreq = IpcService::decode_ipc_req(bytes.to_vec())?;
                            IpcService::handle_ipc_request(&device_tx, &clipboard_tx, ipcreq, &mut ws).await?;
                        }
                        _ => break,
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_ipc_request(
        device_tx: &UnboundedSender<DeviceCommands>,
        clipboard_tx: &UnboundedSender<ClipboardCommand>,
        ipcreq: clipx::IpcRequest,
        ws: &mut WebSocketStream<LocalStream>,
    ) -> anyhow::Result<()> {
        let req = match ipcreq.request {
            Some(req) => req,
            None => return Err(anyhow::anyhow!("Invalid Ipc request")),
        };

        let response = match req {
            clipx::ipc_request::Request::Seen(clipx::SeenRequest { mode }) => {
                use clipx::seen_request::Mode;
                let mode = Mode::try_from(mode)?;
                let seen_mode = match mode {
                    Mode::All => SeenMode::All,
                    Mode::Trusted => SeenMode::Trusted,
                    Mode::Untrusted => SeenMode::Untrusted,
                };
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::GetSeen {
                    mode: seen_mode,
                    reply_to: cmdres_tx,
                });
                let devices = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Seen(clipx::SeenResponse {
                        devices: devices
                            .into_iter()
                            .map(|device| clipx::DeviceInfo {
                                id: device.id,
                                name: device.name,
                                device_type: device.device_type as i32,
                                address: device.addr.to_string(),
                                last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                                ws_port: device.ws_port,
                                connection: clipx::ConnectionState::Disconnected as i32,
                                auto_connect: false,
                                trusted: false,
                            })
                            .collect(),
                    })),
                }
            }
            clipx::ipc_request::Request::Trusted(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::GetSeen {
                    mode: SeenMode::Trusted,
                    reply_to: cmdres_tx,
                });
                let devices = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Trusted(
                        clipx::TrustedResponse {
                            devices: devices
                                .into_iter()
                                .map(|device| clipx::DeviceInfo {
                                    id: device.id,
                                    name: device.name,
                                    device_type: device.device_type as i32,
                                    address: device.addr.to_string(),
                                    last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                                    ws_port: device.ws_port,
                                    connection: clipx::ConnectionState::Disconnected as i32,
                                    auto_connect: false,
                                    trusted: false,
                                })
                                .collect(),
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::Identity(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::GetIdentity {
                    reply_to: cmdres_tx,
                });
                let snapshot = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Identity(
                        clipx::IdentityResponse {
                            device_id: snapshot.device_id,
                            device_name: snapshot.device_name,
                            public_key_hex: snapshot.public_key_hex,
                            ws_port: snapshot.ws_port,
                            device_type: snapshot.device_type as i32,
                            ip_address: snapshot.ip_addr,
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::Pair(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Pair {
                    device_id: device_id.clone(),
                    reply_to: cmdres_tx,
                });
                let message = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Pair(clipx::PairResponse {
                        device_id,
                        status: "ok".to_string(),
                        message,
                        pairing_code: String::new(),
                    })),
                }
            }
            clipx::ipc_request::Request::PairPending(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::PendingPairings {
                    reply_to: cmdres_tx,
                });
                let pending = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::PairPending(
                        clipx::PairPendingResponse {
                            pending: pending
                                .into_iter()
                                .map(|p| clipx::PairPendingSession {
                                    device_id: p.device_id,
                                    device_name: p.device_name,
                                    stage: p.stage.to_string(),
                                    code: p.code.unwrap_or_default(),
                                })
                                .collect(),
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::PairApprove(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::ApprovePairing {
                    device_id: device_id.clone(),
                    approve: req.approve,
                    reply_to: cmdres_tx,
                });
                let message = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::PairApprove(
                        clipx::PairApproveResponse {
                            device_id,
                            status: "ok".to_string(),
                            message,
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::Connect(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connect {
                    device_id: device_id.clone(),
                    reply_to: cmdres_tx,
                });
                let message = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Connect(
                        clipx::ConnectResponse {
                            device_id,
                            status: "ok".to_string(),
                            message,
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::Connected(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected {
                    reply_to: cmdres_tx,
                });
                let devices = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Connected(
                        clipx::ConnectedResponse {
                            devices: devices
                                .into_iter()
                                .map(|device| clipx::DeviceInfo {
                                    id: device.id,
                                    name: device.name,
                                    device_type: device.device_type as i32,
                                    address: device.addr.to_string(),
                                    last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                                    ws_port: device.ws_port,
                                    connection: clipx::ConnectionState::Disconnected as i32,
                                    auto_connect: false,
                                    trusted: false,
                                })
                                .collect(),
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::GetPaired(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::GetPaired {
                    reply_to: cmdres_tx,
                });
                let devices = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::GetPaired(
                        clipx::GetPairedResponse { devices },
                    )),
                }
            }
            clipx::ipc_request::Request::Disconnect(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Disconnect {
                    device_id: device_id.clone(),
                    reply_to: cmdres_tx,
                });
                let message = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Disconnect(
                        clipx::DisconnectResponse {
                            device_id,
                            status: "ok".to_string(),
                            message,
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::SetAutoConnect(req) => {
                let device_id = req.device_id.clone();
                let auto_connect = req.auto_connect;
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::SetAutoConnect {
                    device_id: device_id.clone(),
                    auto_connect,
                    reply_to: cmdres_tx,
                });
                let ok = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::SetAutoConnect(
                        clipx::SetAutoConnectResponse {
                            device_id,
                            auto_connect: if ok { auto_connect } else { !auto_connect }, // report what actually took effect
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::ForgetDevice(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::ForgetDevice {
                    device_id: device_id.clone(),
                    reply_to: cmdres_tx,
                });
                let status = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ForgetDevice(
                        clipx::ForgetDeviceResponse { device_id, status },
                    )),
                }
            }
            clipx::ipc_request::Request::ClipboardHistory(req) => {
                let limit = if req.limit == 0 {
                    None
                } else {
                    Some(req.limit as usize)
                };
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = clipboard_tx.send(ClipboardCommand::GetHistory {
                    limit,
                    reply_to: cmdres_tx,
                });
                let items = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ClipboardHistory(
                        clipx::ClipboardHistoryResponse {
                            entries: items
                                .into_iter()
                                .map(|item| clipx::ClipHistoryEntry {
                                    id: item.id,
                                    content: item.content,
                                    source_device_name: item.source_device,
                                    received_at_ms: item.received_at_ms,
                                })
                                .collect(),
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::ClipboardRemove(req) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = clipboard_tx.send(ClipboardCommand::RemoveEntry {
                    id: req.id,
                    reply_to: cmdres_tx,
                });
                let removed = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ClipboardRemove(
                        clipx::ClipboardRemoveResponse { removed },
                    )),
                }
            }
            clipx::ipc_request::Request::ClipboardClear(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = clipboard_tx.send(ClipboardCommand::ClearHistory {
                    reply_to: cmdres_tx,
                });
                let _ = cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ClipboardClear(
                        clipx::ClipboardClearResponse {},
                    )),
                }
            }
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        ws.send(WsMessage::Binary(buf.into())).await?;
        Ok(())
    }
}

impl IpcService {
    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    fn decode_ipc_req(bytes: Vec<u8>) -> anyhow::Result<clipx::IpcRequest> {
        Ok(clipx::IpcRequest::decode(bytes.as_slice())?)
    }
}
