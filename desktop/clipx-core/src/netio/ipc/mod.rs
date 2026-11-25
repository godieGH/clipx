use crate::{
    clipboard::manager::ClipboardCommand,
    device::manager::{DeviceCommands, SeenMode},
    message::proto::clipx,
    platform::{CoreEvent, PairingEventState},
};
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{
    ListenerOptions,
    tokio::{Listener, Stream as LocalStream},
    traits::tokio::Listener as ListenerTrait,
};
use prost::Message;
use std::{sync::Arc, time::Duration, time::Instant};
use tokio::sync::{broadcast, mpsc::UnboundedSender, oneshot, watch};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async,
    tungstenite::{
        Message as WsMessage,
        handshake::server::{Request, Response},
    },
};
use tokio_util::task::TaskTracker;

mod auth;
pub mod endpoint;
mod notification_center;
use crate::notification::ipc_engine::{AttachGuard, HubEvent, PromptHub};

pub struct IpcService {
    name: String,
    listener: Option<Listener>,
    core_service_shutdown_rx: watch::Receiver<bool>,
    device_tx: UnboundedSender<DeviceCommands>,
    clipboard_tx: UnboundedSender<ClipboardCommand>,
    events_tx: broadcast::Sender<CoreEvent>,
    tracker: TaskTracker,
    stop_tx: UnboundedSender<()>,
    prompt_hub: PromptHub,
}

/// A connection that asked to show prompts itself (see `AttachNotificationCenter`).
struct Attached {
    _guard: AttachGuard,
    events: broadcast::Receiver<HubEvent>,
}

impl IpcService {
    pub fn new(
        name: impl Into<String>,
        shutdown_rx: watch::Receiver<bool>,
        device_tx: UnboundedSender<DeviceCommands>,
        clipboard_tx: UnboundedSender<ClipboardCommand>,
        events_tx: broadcast::Sender<CoreEvent>,
        stop_tx: UnboundedSender<()>,
        prompt_hub: PromptHub,
    ) -> Self {
        Self {
            name: name.into(),
            listener: None,
            core_service_shutdown_rx: shutdown_rx,
            device_tx,
            clipboard_tx,
            events_tx,
            tracker: TaskTracker::new(),
            stop_tx,
            prompt_hub,
        }
    }

    /// Same shape as before — spawn/await this like any other core task.
    /// No caller-side changes needed.
    pub async fn start(&mut self) {
        let (listener, token) = match self.bind() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "IPC endpoint could not be created securely; stopping core");
                let _ = self.stop_tx.send(());
                return;
            }
        };
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
                        tracing::debug!("local IPC client accepted");
                        let device_tx = self.device_tx.clone();
                        let clipboard_tx = self.clipboard_tx.clone();
                        let events_tx = self.events_tx.clone();
                        let client_shutdown_rx = self.core_service_shutdown_rx.clone();
                        let stop_tx = self.stop_tx.clone();
                        let prompt_hub = self.prompt_hub.clone();
                        let token = token.clone();
                        self.tracker.spawn(async move {
                            if let Err(err) =
                                IpcService::handle_client(stream, device_tx, clipboard_tx, events_tx, stop_tx, prompt_hub, token, client_shutdown_rx).await
                            {
                                tracing::warn!(error = %err, "IPC client handler ended with an error");
                            }
                        });
                    }
                        Err(e) => tracing::warn!(error = %e, "IPC accept failed"),
                    }
                }
            }
        }

        // Stop accepting new tasks into the tracker, then wait for every
        // in-flight client task to actually finish (each is racing the same
        // shutdown signal internally, so this resolves promptly).
        self.tracker.close();
        self.tracker.wait().await;
        auth::remove_token(&self.name);
        tracing::info!("All IPC client connections closed");
    }

    /// Issues the session token first, then creates the endpoint restricted to
    /// the current user, so any client that can connect can read the token.
    fn bind(&self) -> anyhow::Result<(Listener, Arc<str>)> {
        let token = auth::issue_token(&self.name)?;
        #[cfg(unix)]
        let _ = std::fs::remove_file(endpoint::socket_path(&self.name)?); // stale socket from a crash
        let opts = ListenerOptions::new().name(endpoint::endpoint_name(&self.name)?);
        #[cfg(windows)]
        let opts = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            opts.security_descriptor(auth::current_user_sd()?)
        };
        let listener = opts.create_tokio()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                endpoint::socket_path(&self.name)?,
                std::fs::Permissions::from_mode(0o600),
            )?;
        }
        Ok((listener, Arc::from(token)))
    }

    // The handshake callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    async fn handle_client(
        conn: LocalStream,
        device_tx: UnboundedSender<DeviceCommands>,
        clipboard_tx: UnboundedSender<ClipboardCommand>,
        events_tx: broadcast::Sender<CoreEvent>,
        stop_tx: UnboundedSender<()>,
        prompt_hub: PromptHub,
        token: Arc<str>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        // No valid token within 5s: dropped before any request is parsed.
        let mut ws = tokio::time::timeout(
            Duration::from_secs(5),
            accept_hdr_async(conn, |req: &Request, resp: Response| {
                auth::check(req, &token).map(|_| resp)
            }),
        )
        .await??;
        let mut events_rx = events_tx.subscribe();
        // Set once this connection asks to show prompts. `announced` holds the
        // prompts already sent from the pending snapshot, so one that also
        // arrives through the live feed is not sent twice.
        let mut attached: Option<Attached> = None;
        let mut announced: std::collections::HashSet<String> = std::collections::HashSet::new();
        tracing::debug!("IPC client session started");

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        let _ = ws.close(None).await;
                        break;
                    }
                }
                // Unsolicited push — goes out on this client's socket the
                // moment DeviceManager/ClipboardManager fires it. Lagged
                // just means this slow client missed some pings; since
                // every event is a payload-free "go re-fetch" signal, the
                // next one (or the client's own next request) catches it
                // up fine — no need to treat it as fatal.
                event = events_rx.recv() => {
                    match event {
                        Ok(event) => {
                            let event_kind = match &event {
                                CoreEvent::DevicesChanged => "devices_changed",
                                CoreEvent::ClipboardChanged => "clipboard_changed",
                                CoreEvent::PairingChanged { .. } => "pairing_changed",
                                CoreEvent::FileTransferChanged { .. } => "file_transfer_changed",
                            };
                            let event = match event {
                                CoreEvent::DevicesChanged => clipx::IpcEvent {
                                    event: Some(clipx::ipc_event::Event::DevicesChanged(clipx::DevicesChangedEvent {})),
                                },
                                CoreEvent::ClipboardChanged => clipx::IpcEvent {
                                    event: Some(clipx::ipc_event::Event::ClipboardChanged(clipx::ClipboardChangedEvent {})),
                                },
                                CoreEvent::PairingChanged { device_id, state, message } => {
                                    let state = match state {
                                        PairingEventState::Started => clipx::pairing_event::State::Started,
                                        PairingEventState::Failed => clipx::pairing_event::State::Failed,
                                        PairingEventState::Succeeded => clipx::pairing_event::State::Succeeded,
                                    };
                                    clipx::IpcEvent {
                                        event: Some(clipx::ipc_event::Event::PairingEvent(clipx::PairingEvent {
                                            device_id,
                                            state: state as i32,
                                            message,
                                        })),
                                    }
                                }
                                CoreEvent::FileTransferChanged { entry_id, file_id, file_name, direction, done, total, state, message, device_id, device_name, can_pause } => clipx::IpcEvent {
                                    event: Some(clipx::ipc_event::Event::FileTransfer(clipx::FileTransferEvent { entry_id, done, total, state, message, file_id, file_name, direction, device_id, device_name, can_pause })),
                                },
                            };
                            let envelope = clipx::IpcServerMessage {
                                payload: Some(clipx::ipc_server_message::Payload::Event(event)),
                            };
                            let mut buf = Vec::new();
                            envelope.encode(&mut buf)?;
                            let event_bytes = buf.len();
                            ws.send(WsMessage::Binary(buf.into())).await?;
                            tracing::trace!(event = event_kind, bytes = event_bytes, "IPC event pushed to client");
                        }
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            tracing::debug!(missed_events = count, "IPC client event receiver lagged");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {}
                    }
                }
                hub_event = async {
                    match attached.as_mut() {
                        Some(a) => a.events.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match hub_event {
                        Ok(HubEvent::Prompt(info)) => {
                            if !announced.remove(&info.prompt_id) {
                                IpcService::send_event(&mut ws, notification_center::prompt_to_ipc(info)).await?;
                            }
                        }
                        Ok(event) => {
                            IpcService::send_event(&mut ws, notification_center::hub_event_to_ipc(event)).await?;
                        }
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            // Events were missed. Prompts still waiting are sent
                            // again; the UI ignores ones it already shows.
                            tracing::warn!(missed_events = count, "notification center lagged; resending pending prompts");
                            for info in prompt_hub.pending() {
                                IpcService::send_event(&mut ws, notification_center::prompt_to_ipc(info)).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => attached = None,
                    }
                }
                msg = ws.next() => {
                    let Some(msg) = msg else { break };
                    let msg = msg?;
                    match msg {
                        WsMessage::Close(_) => break,
                        WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
                        WsMessage::Binary(bytes) => {
                            tracing::trace!(bytes = bytes.len(), "IPC request frame received");
                            let ipcreq = IpcService::decode_ipc_req(bytes.to_vec())?;
                            if matches!(ipcreq.request, Some(clipx::ipc_request::Request::AttachNotificationCenter(_))) {
                                // Per-connection state, so it is handled here
                                // rather than in the stateless dispatcher.
                                let pending = match attached {
                                    Some(_) => Vec::new(),
                                    None => {
                                        let (guard, events, pending) = prompt_hub.attach();
                                        attached = Some(Attached { _guard: guard, events });
                                        announced.extend(pending.iter().map(|p| p.prompt_id.clone()));
                                        pending
                                    }
                                };
                                tracing::debug!(pending = pending.len(), "notification center attached");
                                IpcService::send_response(&mut ws, clipx::IpcResponse {
                                    response: Some(clipx::ipc_response::Response::AttachNotificationCenter(
                                        clipx::AttachNotificationCenterResponse { pending: pending.len() as u32 },
                                    )),
                                }).await?;
                                for info in pending {
                                    IpcService::send_event(&mut ws, notification_center::prompt_to_ipc(info)).await?;
                                }
                                continue;
                            }
                            IpcService::handle_ipc_request(&device_tx, &clipboard_tx, &stop_tx, &prompt_hub, ipcreq, &mut ws).await?;
                        }
                        _ => break,
                    }
                }
            }
        }

        tracing::debug!("IPC client session closed");
        Ok(())
    }

    async fn handle_ipc_request(
        device_tx: &UnboundedSender<DeviceCommands>,
        clipboard_tx: &UnboundedSender<ClipboardCommand>,
        stop_tx: &UnboundedSender<()>,
        prompt_hub: &PromptHub,
        ipcreq: clipx::IpcRequest,
        ws: &mut WebSocketStream<LocalStream>,
    ) -> anyhow::Result<()> {
        let request_started = Instant::now();
        let req = match ipcreq.request {
            Some(req) => req,
            None => return Err(anyhow::anyhow!("Invalid Ipc request")),
        };
        let request_kind = match &req {
            clipx::ipc_request::Request::Seen(_) => "seen",
            clipx::ipc_request::Request::Trusted(_) => "trusted",
            clipx::ipc_request::Request::Identity(_) => "identity",
            clipx::ipc_request::Request::Pair(_) => "pair",
            clipx::ipc_request::Request::PairPending(_) => "pair_pending",
            clipx::ipc_request::Request::PairApprove(_) => "pair_approve",
            clipx::ipc_request::Request::Connect(_) => "connect",
            clipx::ipc_request::Request::Connected(_) => "connected",
            clipx::ipc_request::Request::GetPaired(_) => "get_paired",
            clipx::ipc_request::Request::Disconnect(_) => "disconnect",
            clipx::ipc_request::Request::SetAutoConnect(_) => "set_auto_connect",
            clipx::ipc_request::Request::ForgetDevice(_) => "forget_device",
            clipx::ipc_request::Request::ClipboardHistory(_) => "clipboard_history",
            clipx::ipc_request::Request::ClipboardRemove(_) => "clipboard_remove",
            clipx::ipc_request::Request::ClipboardClear(_) => "clipboard_clear",
            clipx::ipc_request::Request::SendText(_) => "send_text",
            clipx::ipc_request::Request::SendRichText(_) => "send_rich_text",
            clipx::ipc_request::Request::SendImage(_) => "send_image",
            clipx::ipc_request::Request::SendFile(_) => "send_file",
            clipx::ipc_request::Request::SendFilePath(_) => "send_file_path",
            clipx::ipc_request::Request::DownloadClipboardFile(_) => "download_clipboard_file",
            clipx::ipc_request::Request::CopyClipboardEntry(_) => "copy_clipboard_entry",
            clipx::ipc_request::Request::Shutdown(_) => "shutdown",
            clipx::ipc_request::Request::AttachNotificationCenter(_) => {
                "attach_notification_center"
            }
            clipx::ipc_request::Request::ResolvePrompt(_) => "resolve_prompt",
            clipx::ipc_request::Request::ControlTransfer(_) => "control_transfer",
            clipx::ipc_request::Request::ListTransfers(_) => "list_transfers",
        };
        tracing::trace!(request = request_kind, "IPC request dispatch started");

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
                                    kind: match item.kind {
                                        crate::clipboard::clipstore::ClipKind::Text => "text",
                                        crate::clipboard::clipstore::ClipKind::RichText => {
                                            "rich_text"
                                        }
                                        crate::clipboard::clipstore::ClipKind::Image => "image",
                                        crate::clipboard::clipstore::ClipKind::File => "file",
                                    }
                                    .into(),
                                    html: item.html.unwrap_or_default(),
                                    file_id: item.file_id.unwrap_or_default(),
                                    file_name: item.file_name.unwrap_or_default(),
                                    mime_type: item.mime_type.unwrap_or_default(),
                                    file_size: item.file_size,
                                    file_expires_at_ms: item.file_expires_at_ms,
                                    file_downloaded: item.file_downloaded,
                                    local_file_path: item.local_file_path.unwrap_or_default(),
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
                cmdres_rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ClipboardClear(
                        clipx::ClipboardClearResponse {},
                    )),
                }
            }
            clipx::ipc_request::Request::SendText(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected { reply_to: tx });
                if rx.await?.is_empty() {
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: false,
                            message: "No connected devices".into(),
                        })),
                    }
                } else {
                    let _ = clipboard_tx.send(ClipboardCommand::SendLocal {
                        payload: crate::clipboard::manager::ClipboardPayload::Text(req.content),
                    });
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: true,
                            message: "sent".into(),
                        })),
                    }
                }
            }
            clipx::ipc_request::Request::SendRichText(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected { reply_to: tx });
                if rx.await?.is_empty() {
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: false,
                            message: "No connected devices".into(),
                        })),
                    }
                } else {
                    let _ = clipboard_tx.send(ClipboardCommand::SendLocal {
                        payload: crate::clipboard::manager::ClipboardPayload::RichText {
                            text: req.text,
                            html: req.html,
                        },
                    });
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: true,
                            message: "sent".into(),
                        })),
                    }
                }
            }
            clipx::ipc_request::Request::SendImage(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected { reply_to: tx });
                if rx.await?.is_empty() {
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: false,
                            message: "No connected devices".into(),
                        })),
                    }
                } else {
                    let _ = clipboard_tx.send(ClipboardCommand::SendLocal {
                        payload: crate::clipboard::manager::ClipboardPayload::Image {
                            width: req.width,
                            height: req.height,
                            rgba: req.rgba,
                        },
                    });
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: true,
                            message: "sent".into(),
                        })),
                    }
                }
            }
            clipx::ipc_request::Request::SendFile(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected { reply_to: tx });
                if rx.await?.is_empty() {
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: false,
                            message: "No connected devices".into(),
                        })),
                    }
                } else {
                    let _ = clipboard_tx.send(ClipboardCommand::SendLocal {
                        payload: crate::clipboard::manager::ClipboardPayload::FileBytes {
                            name: req.name,
                            mime_type: req.mime_type,
                            data: req.data,
                        },
                    });
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: true,
                            message: "file offer sent".into(),
                        })),
                    }
                }
            }
            clipx::ipc_request::Request::SendFilePath(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = device_tx.send(DeviceCommands::Connected { reply_to: tx });
                if rx.await?.is_empty() {
                    clipx::IpcResponse {
                        response: Some(clipx::ipc_response::Response::Send(clipx::SendResponse {
                            sent: false,
                            message: "No connected devices".into(),
                        })),
                    }
                } else {
                    let (reply_tx, reply_rx) = oneshot::channel();
                    clipboard_tx
                        .send(ClipboardCommand::SendFilePath {
                            name: req.name,
                            mime_type: req.mime_type,
                            path: std::path::PathBuf::from(req.path),
                            take_ownership: false,
                            reply_to: reply_tx,
                        })
                        .map_err(|_| anyhow::anyhow!("clipboard manager stopped"))?;
                    match reply_rx
                        .await
                        .map_err(|_| anyhow::anyhow!("clipboard manager stopped"))?
                    {
                        Ok(()) => clipx::IpcResponse {
                            response: Some(clipx::ipc_response::Response::Send(
                                clipx::SendResponse {
                                    sent: true,
                                    message: "file offer sent".into(),
                                },
                            )),
                        },
                        Err(message) => clipx::IpcResponse {
                            response: Some(clipx::ipc_response::Response::Send(
                                clipx::SendResponse {
                                    sent: false,
                                    message,
                                },
                            )),
                        },
                    }
                }
            }
            clipx::ipc_request::Request::DownloadClipboardFile(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = clipboard_tx.send(ClipboardCommand::DownloadHistoryFile {
                    entry_id: req.entry_id,
                    reply_to: tx,
                });
                let message = rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::DownloadClipboardFile(
                        clipx::DownloadClipboardFileResponse {
                            started: message == "download requested",
                            message,
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::CopyClipboardEntry(req) => {
                let (tx, rx) = oneshot::channel();
                let _ = clipboard_tx.send(ClipboardCommand::CopyHistoryEntry {
                    id: req.id,
                    reply_to: tx,
                });
                let result = rx.await?;
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::CopyClipboardEntry(
                        clipx::ClipboardCopyResponse {
                            copied: result.is_ok(),
                            message: result.err().unwrap_or_else(|| "copied".into()),
                        },
                    )),
                }
            }
            clipx::ipc_request::Request::AttachNotificationCenter(_) => {
                // Attaching is per connection and is answered before dispatch;
                // reaching here would be a bug in the caller.
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Error(clipx::ErrorResponse {
                        message: "attach_notification_center is handled per connection".into(),
                    })),
                }
            }
            clipx::ipc_request::Request::ResolvePrompt(req) => clipx::IpcResponse {
                response: Some(clipx::ipc_response::Response::ResolvePrompt(
                    notification_center::resolve_prompt_response(prompt_hub, req),
                )),
            },
            clipx::ipc_request::Request::ControlTransfer(req) => clipx::IpcResponse {
                response: Some(clipx::ipc_response::Response::ControlTransfer(
                    notification_center::control_transfer_response(device_tx, req).await,
                )),
            },
            clipx::ipc_request::Request::ListTransfers(_) => clipx::IpcResponse {
                response: Some(clipx::ipc_response::Response::ListTransfers(
                    notification_center::list_transfers_response(device_tx).await,
                )),
            },
            clipx::ipc_request::Request::Shutdown(_) => {
                let _ = stop_tx.send(());
                clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::Shutdown(
                        clipx::ShutdownResponse {},
                    )),
                }
            }
        };

        let envelope = clipx::IpcServerMessage {
            payload: Some(clipx::ipc_server_message::Payload::Response(response)),
        };

        let mut buf = Vec::new();
        envelope.encode(&mut buf)?;
        let response_bytes = buf.len();
        ws.send(WsMessage::Binary(buf.into())).await?;
        tracing::trace!(
            request = request_kind,
            response_bytes,
            elapsed_us = request_started.elapsed().as_micros() as u64,
            "IPC response sent"
        );
        Ok(())
    }
}

impl IpcService {
    async fn send_response(
        ws: &mut WebSocketStream<LocalStream>,
        response: clipx::IpcResponse,
    ) -> anyhow::Result<()> {
        let envelope = clipx::IpcServerMessage {
            payload: Some(clipx::ipc_server_message::Payload::Response(response)),
        };
        let mut buf = Vec::new();
        envelope.encode(&mut buf)?;
        ws.send(WsMessage::Binary(buf.into())).await?;
        Ok(())
    }

    async fn send_event(
        ws: &mut WebSocketStream<LocalStream>,
        event: clipx::IpcEvent,
    ) -> anyhow::Result<()> {
        let envelope = clipx::IpcServerMessage {
            payload: Some(clipx::ipc_server_message::Payload::Event(event)),
        };
        let mut buf = Vec::new();
        envelope.encode(&mut buf)?;
        ws.send(WsMessage::Binary(buf.into())).await?;
        Ok(())
    }

    fn decode_ipc_req(bytes: Vec<u8>) -> anyhow::Result<clipx::IpcRequest> {
        let wire_bytes = bytes.len();
        match clipx::IpcRequest::decode(bytes.as_slice()) {
            Ok(request) => {
                tracing::trace!(wire_bytes, "IPC request protobuf decoded");
                Ok(request)
            }
            Err(error) => {
                tracing::warn!(wire_bytes, %error, "invalid IPC request protobuf");
                Err(error.into())
            }
        }
    }
}

#[cfg(test)]
mod socket_tests {
    //! The notification center end to end over a real local socket: the same
    //! framing a UI uses, a real `IpcService`, and a wrapped engine standing in
    //! for the native one.
    use super::*;
    use crate::notification::{
        IncomingClipboardDecision, PairDecision, Prompt,
        ipc_engine::IpcNotificationEngine,
        platform::{NotificationEngine as Engine, NotificationFuture},
    };
    use interprocess::local_socket::ConnectOptions;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };
    use tokio::{sync::mpsc, time::timeout};
    use tokio_tungstenite::{
        client_async,
        tungstenite::{client::IntoClientRequest, http::HeaderValue},
    };

    /// The "native" engine: counts how often it is asked and refuses everything.
    #[derive(Clone, Default)]
    struct Native {
        pairs: Arc<AtomicU32>,
    }

    impl Engine for Native {
        fn ask_pair<'a>(&'a self, _: Prompt, _: Duration) -> NotificationFuture<'a, PairDecision> {
            self.pairs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { PairDecision::Deny })
        }
        fn ask_clipboard<'a>(
            &'a self,
            _: Prompt,
            _: Duration,
        ) -> NotificationFuture<'a, IncomingClipboardDecision> {
            Box::pin(async { IncomingClipboardDecision::Ignore })
        }
        fn notify_info<'a>(&'a self, _: &'a str, _: String) -> NotificationFuture<'a, ()> {
            Box::pin(async {})
        }
    }

    type Ws = WebSocketStream<LocalStream>;

    struct Rig {
        name: String,
        engine: IpcNotificationEngine<Native>,
        native: Native,
        hub: PromptHub,
        shutdown_tx: watch::Sender<bool>,
        service: tokio::task::JoinHandle<()>,
        device_rx: Option<mpsc::UnboundedReceiver<DeviceCommands>>,
    }

    async fn rig() -> Rig {
        let name = format!("clipx-test-{}", uuid::Uuid::new_v4());
        let native = Native::default();
        let engine = IpcNotificationEngine::new(native.clone());
        let hub = engine.hub();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (device_tx, device_rx) = mpsc::unbounded_channel();
        let (clipboard_tx, _clipboard_rx) = mpsc::unbounded_channel();
        let (events_tx, _) = broadcast::channel(8);
        let (stop_tx, _stop_rx) = mpsc::unbounded_channel();
        let mut service = IpcService::new(
            name.clone(),
            shutdown_rx,
            device_tx,
            clipboard_tx,
            events_tx,
            stop_tx,
            hub.clone(),
        );
        let service = tokio::spawn(async move { service.start().await });
        Rig {
            name,
            engine,
            native,
            hub,
            shutdown_tx,
            service,
            device_rx: Some(device_rx),
        }
    }

    impl Rig {
        async fn connect(&self) -> Ws {
            for _ in 0..100 {
                let name = endpoint::endpoint_name(&self.name).unwrap();
                if let Ok(conn) = ConnectOptions::new().name(name).connect_tokio().await {
                    let mut req = "ws://localhost/".into_client_request().unwrap();
                    req.headers_mut().insert(
                        "Authorization",
                        HeaderValue::from_str(&endpoint::bearer(&self.name).unwrap()).unwrap(),
                    );
                    let (ws, _) = client_async(req, conn).await.unwrap();
                    return ws;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("could not connect to the test IPC service");
        }

        async fn stop(self) {
            let _ = self.shutdown_tx.send(true);
            let _ = timeout(Duration::from_secs(3), self.service).await;
        }
    }

    enum Frame {
        Response(clipx::IpcResponse),
        Event(clipx::IpcEvent),
    }

    async fn request(ws: &mut Ws, request: clipx::ipc_request::Request) {
        let mut buf = Vec::new();
        clipx::IpcRequest {
            request: Some(request),
        }
        .encode(&mut buf)
        .unwrap();
        ws.send(WsMessage::Binary(buf.into())).await.unwrap();
    }

    async fn next_frame(ws: &mut Ws) -> Option<Frame> {
        let msg = timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        let WsMessage::Binary(bytes) = msg else {
            return None;
        };
        match clipx::IpcServerMessage::decode(bytes.as_ref())
            .ok()?
            .payload?
        {
            clipx::ipc_server_message::Payload::Response(r) => Some(Frame::Response(r)),
            clipx::ipc_server_message::Payload::Event(e) => Some(Frame::Event(e)),
        }
    }

    /// True if nothing arrives for a short while.
    async fn quiet(ws: &mut Ws) -> bool {
        timeout(Duration::from_millis(250), ws.next())
            .await
            .is_err()
    }

    async fn expect_response(ws: &mut Ws) -> clipx::IpcResponse {
        loop {
            match next_frame(ws).await.expect("a frame") {
                Frame::Response(r) => return r,
                Frame::Event(_) => continue,
            }
        }
    }

    async fn expect_prompt(ws: &mut Ws) -> clipx::NotificationPromptEvent {
        loop {
            match next_frame(ws).await.expect("a frame") {
                Frame::Event(clipx::IpcEvent {
                    event: Some(clipx::ipc_event::Event::NotificationPrompt(p)),
                }) => return p,
                _ => continue,
            }
        }
    }

    fn attach() -> clipx::ipc_request::Request {
        clipx::ipc_request::Request::AttachNotificationCenter(
            clipx::AttachNotificationCenterRequest {},
        )
    }

    fn pair_prompt() -> Prompt {
        Prompt::PairRequest {
            peer_name: "Pixel".into(),
        }
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..100 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("condition not met in time");
    }

    #[tokio::test]
    async fn an_attached_ui_gets_a_prompt_answers_it_and_sees_it_close() {
        let rig = rig().await;
        let mut ui = rig.connect().await;

        request(&mut ui, attach()).await;
        match expect_response(&mut ui).await.response {
            Some(clipx::ipc_response::Response::AttachNotificationCenter(r)) => {
                assert_eq!(r.pending, 0)
            }
            other => panic!("unexpected {other:?}"),
        }

        let asked = tokio::spawn({
            let engine = rig.engine.clone();
            async move {
                engine
                    .ask_pair(pair_prompt(), Duration::from_secs(10))
                    .await
            }
        });
        let prompt = expect_prompt(&mut ui).await;
        assert_eq!(prompt.peer_name, "Pixel");
        assert_eq!(
            prompt.kind,
            clipx::notification_prompt_event::Kind::PairRequest as i32
        );

        request(
            &mut ui,
            clipx::ipc_request::Request::ResolvePrompt(clipx::ResolvePromptRequest {
                prompt_id: prompt.prompt_id.clone(),
                decision: clipx::resolve_prompt_request::Decision::Allow as i32,
            }),
        )
        .await;
        let mut resolved = false;
        let mut closed = false;
        while !(resolved && closed) {
            match next_frame(&mut ui).await.expect("a frame") {
                Frame::Response(clipx::IpcResponse {
                    response: Some(clipx::ipc_response::Response::ResolvePrompt(r)),
                }) => resolved = r.resolved,
                Frame::Event(clipx::IpcEvent {
                    event: Some(clipx::ipc_event::Event::NotificationClosed(c)),
                }) => {
                    assert_eq!(c.prompt_id, prompt.prompt_id);
                    assert_eq!(
                        c.reason,
                        clipx::notification_closed_event::Reason::Answered as i32
                    );
                    closed = true;
                }
                _ => {}
            }
        }
        assert_eq!(asked.await.unwrap(), PairDecision::Allow);
        assert_eq!(
            rig.native.pairs.load(Ordering::SeqCst),
            0,
            "the native engine was not used"
        );
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_ui_that_attaches_late_gets_each_waiting_prompt_exactly_once() {
        let rig = rig().await;
        let mut first = rig.connect().await;
        request(&mut first, attach()).await;
        let _ = expect_response(&mut first).await;

        let asked = tokio::spawn({
            let engine = rig.engine.clone();
            async move {
                engine
                    .ask_pair(pair_prompt(), Duration::from_secs(10))
                    .await
            }
        });
        let seen_by_first = expect_prompt(&mut first).await;
        drop(first);
        wait_until(|| !rig.hub.has_attached_ui()).await;

        // The prompt is still waiting; a UI that comes up now is shown it.
        let mut late = rig.connect().await;
        request(&mut late, attach()).await;
        match expect_response(&mut late).await.response {
            Some(clipx::ipc_response::Response::AttachNotificationCenter(r)) => {
                assert_eq!(r.pending, 1)
            }
            other => panic!("unexpected {other:?}"),
        }
        let replayed = expect_prompt(&mut late).await;
        assert_eq!(replayed.prompt_id, seen_by_first.prompt_id);
        assert!(
            quiet(&mut late).await,
            "the same prompt must not be sent twice"
        );

        request(
            &mut late,
            clipx::ipc_request::Request::ResolvePrompt(clipx::ResolvePromptRequest {
                prompt_id: replayed.prompt_id,
                decision: clipx::resolve_prompt_request::Decision::Deny as i32,
            }),
        )
        .await;
        assert_eq!(asked.await.unwrap(), PairDecision::Deny);
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_ui_that_never_attached_receives_no_prompts_and_the_native_engine_is_used() {
        let rig = rig().await;
        let mut old_ui = rig.connect().await;

        // A connection that only speaks the old protocol: nothing attached, so
        // the prompt goes to the native engine and this connection hears nothing.
        let decision = rig
            .engine
            .ask_pair(pair_prompt(), Duration::from_secs(5))
            .await;
        assert_eq!(decision, PairDecision::Deny);
        assert_eq!(rig.native.pairs.load(Ordering::SeqCst), 1);
        assert!(quiet(&mut old_ui).await);
        rig.stop().await;
    }

    #[tokio::test]
    async fn closing_the_ui_hands_prompts_back_to_the_native_engine() {
        let rig = rig().await;
        let mut ui = rig.connect().await;
        request(&mut ui, attach()).await;
        let _ = expect_response(&mut ui).await;
        assert!(rig.hub.has_attached_ui());

        drop(ui);
        wait_until(|| !rig.hub.has_attached_ui()).await;

        assert_eq!(
            rig.engine
                .ask_pair(pair_prompt(), Duration::from_secs(5))
                .await,
            PairDecision::Deny
        );
        assert_eq!(rig.native.pairs.load(Ordering::SeqCst), 1);
        rig.stop().await;
    }

    #[tokio::test]
    async fn attaching_twice_on_one_connection_counts_once() {
        let rig = rig().await;
        let mut ui = rig.connect().await;
        request(&mut ui, attach()).await;
        let _ = expect_response(&mut ui).await;
        request(&mut ui, attach()).await;
        let _ = expect_response(&mut ui).await;

        drop(ui);
        wait_until(|| !rig.hub.has_attached_ui()).await;
        rig.stop().await;
    }

    #[tokio::test]
    async fn transfer_requests_go_to_the_device_manager_over_the_socket() {
        let mut rig = rig().await;
        let mut device_rx = rig.device_rx.take().unwrap();
        let manager = tokio::spawn(async move {
            while let Some(cmd) = device_rx.recv().await {
                match cmd {
                    DeviceCommands::ControlTransfer {
                        file_id, reply_to, ..
                    } => {
                        let _ = reply_to.send(if file_id == "old-peer" {
                            Err("older ClipX".to_string())
                        } else {
                            Ok("transfer paused".to_string())
                        });
                    }
                    DeviceCommands::ListTransfers { reply_to } => {
                        let _ = reply_to.send(vec![crate::device::manager::TransferSnapshot {
                            device_id: "dev-a".into(),
                            device_name: "Pixel".into(),
                            entry_id: "e1".into(),
                            file_id: "f1".into(),
                            file_name: "movie.mp4".into(),
                            direction: "download".into(),
                            done: 10,
                            total: 100,
                            state: "receiving".into(),
                            can_pause: true,
                        }]);
                    }
                    _ => {}
                }
            }
        });
        let mut ui = rig.connect().await;

        let control = |file: &str| {
            clipx::ipc_request::Request::ControlTransfer(clipx::ControlTransferRequest {
                file_id: file.into(),
                device_id: String::new(),
                action: clipx::control_transfer_request::Action::Pause as i32,
            })
        };
        request(&mut ui, control("f1")).await;
        match expect_response(&mut ui).await.response {
            Some(clipx::ipc_response::Response::ControlTransfer(r)) => {
                assert!(r.ok);
                assert_eq!(r.message, "transfer paused");
            }
            other => panic!("unexpected {other:?}"),
        }
        request(&mut ui, control("old-peer")).await;
        match expect_response(&mut ui).await.response {
            Some(clipx::ipc_response::Response::ControlTransfer(r)) => {
                assert!(!r.ok);
                assert_eq!(r.message, "older ClipX");
            }
            other => panic!("unexpected {other:?}"),
        }

        request(
            &mut ui,
            clipx::ipc_request::Request::ListTransfers(clipx::ListTransfersRequest {}),
        )
        .await;
        match expect_response(&mut ui).await.response {
            Some(clipx::ipc_response::Response::ListTransfers(r)) => {
                assert_eq!(r.transfers.len(), 1);
                assert_eq!(
                    (
                        r.transfers[0].device_name.as_str(),
                        r.transfers[0].done,
                        r.transfers[0].can_pause
                    ),
                    ("Pixel", 10, true)
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        manager.abort();
        rig.stop().await;
    }
}
