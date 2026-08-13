use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, ToNsName,
    tokio::{Listener, Stream as LocalStream},
    traits::tokio::Listener as ListenerTrait,
};
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch};
use tokio_tungstenite::{accept_async, WebSocketStream, tungstenite::Message as WsMessage};
use futures_util::{SinkExt, StreamExt};

use crate::{
    device::{manager::{DeviceCommands, SeenMode}},
    message::proto::clipx,
};
use prost::Message;

// starts and manage ipc resources
pub struct IpcService {
    name: String,
    listener: Option<Listener>,
    core_service_shutdown_rx: watch::Receiver<bool>,
    device_tx: UnboundedSender<DeviceCommands>
}

impl IpcService {
    pub fn new(
        name: impl Into<String>,
        shutdown_rx: watch::Receiver<bool>,
        device_tx: UnboundedSender<DeviceCommands>
    ) -> Self {
        Self {
            name: name.into(),
            listener: None,
            core_service_shutdown_rx: shutdown_rx,
            device_tx,
        }
    }

    pub async fn start(&mut self) {
        let name = self.create_name().unwrap();
        let listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        self.listener = Some(listener);

        tracing::info!("IPC services started");

        loop {
            tokio::select! {
                _ = self.core_service_shutdown_rx.changed() => {
                    if *self.core_service_shutdown_rx.borrow() {
                        break;
                    }
                }
                conn = self.listener.as_ref().unwrap().accept() => {
                    match conn {
                        Ok(stream) => {
                            if let Err(err) = self.handle_client(stream).await {
                                tracing::error!(error = %err, "IPC client handler failed");
                            }
                        }
                        Err(e) => tracing::error!("Accept error: {e:?}"),
                    }
                }
            }
        }
    }

    async fn handle_client(&self, conn: LocalStream) -> anyhow::Result<()> {
        let mut ws = accept_async(conn).await?;

        while let Some(msg) = ws.next().await {
            let msg = msg?;
            if  msg.is_close() {
                break;
            }

            match msg {
                WsMessage::Binary(bytes) => {
                    tracing::info!("Ipc request made");
                    let ipcreq = IpcService::decode_ipc_req(bytes.to_vec())?;
                    self.handle_ipc_request(ipcreq, &mut ws).await?;
                }
                _ => {
                    break;
                }

            }
        }

        // ws.close(None).await?; //this is redundant since peer is one iniating a close and tungestenite sends a closing ack autoamatically
        Ok(())
    }

    async fn handle_ipc_request(
        &self,
        ipcreq: clipx::IpcRequest,
        ws: &mut WebSocketStream<LocalStream>
    ) -> anyhow::Result<()> {
        let req = match ipcreq.request {
            Some(req) => req,
            None => return Err(anyhow::anyhow!("Invalid Ipc request"))
        };

        let response = match req {
            clipx::ipc_request::Request::Seen(clipx::SeenRequest { mode }) => {
                use clipx::seen_request::Mode;
                let mode = Mode::try_from(mode)?;
                let seen_mode = match mode {
                    Mode::All => crate::device::manager::SeenMode::All,
                    Mode::Trusted => crate::device::manager::SeenMode::Trusted,
                    Mode::Untrusted => crate::device::manager::SeenMode::Untrusted,
                };
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::GetSeen { mode: seen_mode, reply_to: cmdres_tx });
                let devices = cmdres_rx.await?;
                let response = clipx::SeenResponse {
                    devices: devices
                        .into_iter()
                        .map(|device| clipx::DeviceInfo {
                            id: device.id,
                            name: device.name,
                            device_type: device.device_type as i32,
                            address: device.addr.to_string(),
                            last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                        })
                        .collect(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Seen(response)),
                }
            }
            clipx::ipc_request::Request::Trusted(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::GetSeen { mode: SeenMode::Trusted, reply_to: cmdres_tx });
                let devices = cmdres_rx.await?;
                let response = clipx::TrustedResponse {
                    devices: devices
                        .into_iter()
                        .map(|device| clipx::DeviceInfo {
                            id: device.id,
                            name: device.name,
                            device_type: device.device_type as i32,
                            address: device.addr.to_string(),
                            last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                        })
                        .collect(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Trusted(response)),
                }
            }
            clipx::ipc_request::Request::Identity(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::GetIdentity { reply_to: cmdres_tx });
                let snapshot = cmdres_rx.await?;
                let response = clipx::IdentityResponse {
                    device_id: snapshot.device_id,
                    device_name: snapshot.device_name,
                    public_key_hex: snapshot.public_key_hex,
                    ws_port: snapshot.ws_port,
                    device_type: snapshot.device_type as i32,
                    ip_address: snapshot.ip_addr
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Identity(response)),
                }
            }
            clipx::ipc_request::Request::Pair(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::Pair { device_id: device_id.clone(), reply_to: cmdres_tx });
                let message = cmdres_rx.await?;
                let response = clipx::PairResponse {
                    device_id,
                    status: "ok".to_string(),
                    message,
                    pairing_code: None.unwrap_or_default(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Pair(response)),
                }
            }
            clipx::ipc_request::Request::PairPending(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::PendingPairings { reply_to: cmdres_tx });
                let pending = cmdres_rx.await?;
                let response = clipx::PairPendingResponse {
                    pending: pending
                        .into_iter()
                        .map(|(device_id, code)| clipx::PairPendingDevice {
                            device_id,
                            device_name: "pending".to_string(),
                            status: code,
                        })
                        .collect(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::PairPending(response)),
                }
            }
            clipx::ipc_request::Request::PairApprove(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::ApprovePairing { device_id: device_id.clone(), reply_to: cmdres_tx });
                let message = cmdres_rx.await?;
                let response = clipx::PairApproveResponse {
                    device_id,
                    status: "ok".to_string(),
                    message,
                    pairing_code: None.unwrap_or_default(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::PairApprove(response)),
                }
            }
            clipx::ipc_request::Request::Connect(req) => {
                let device_id = req.device_id.clone();
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::Connect { device_id: device_id.clone(), reply_to: cmdres_tx });
                let message = cmdres_rx.await?;
                let response = clipx::ConnectResponse {
                    device_id,
                    status: "ok".to_string(),
                    message,
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Connect(response)),
                }
            }
            clipx::ipc_request::Request::Connected(_) => {
                let (cmdres_tx, cmdres_rx) = oneshot::channel();
                let _ = self.device_tx.send(DeviceCommands::Connected { reply_to: cmdres_tx });
                let devices = cmdres_rx.await?;
                let response = clipx::ConnectedResponse {
                    devices: devices
                        .into_iter()
                        .map(|device| clipx::DeviceInfo {
                            id: device.id,
                            name: device.name,
                            device_type: device.device_type as i32,
                            address: device.addr.to_string(),
                            last_seen_ms: device.last_seen.elapsed().as_millis() as u64,
                        })
                        .collect(),
                };
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Connected(response)),
                }
            }
        };

        let mut buf = Vec::new();
        response.encode(&mut buf)?;
        ws.send(WsMessage::Binary(buf.into())).await?;
        Ok(())
    }
}


// other helpers
impl IpcService {
    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    fn decode_ipc_req(bytes: Vec<u8>) -> anyhow::Result<clipx::IpcRequest> {
        Ok(clipx::IpcRequest::decode(bytes.as_slice())?)
    }
}