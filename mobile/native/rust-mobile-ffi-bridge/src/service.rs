use std::sync::Arc;

use clipx_core::device::manager::{DeviceCommands, SeenMode};
use clipx_core::message::proto::{ConnectionState, DeviceType};
use clipx_core::notification::platform::{PlatformNotificationEngine, PromptResult};
use tokio::task::JoinHandle;

use crate::platform::{
    ClipboardPlatform, ClipboardSinkAdapter, ClipxEventListener, NotificationAdapter,
    NotificationPlatform, NotifierDecision,
};

/// Device metadata returned to the host app after the core initializes.
#[derive(uniffi::Record, Clone)]
pub struct MobileIdentity {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub device_type: String,
    pub ip_address: String,
    pub ws_port: u32,
}

/// Peer device information for the paired list presented by the UI.
#[derive(uniffi::Record, Clone)]
pub struct MobilePairedDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub connection: String,
    pub ip_address: String,
    pub ws_port: u32,
    pub auto_connect: bool,
}

/// Remote peer discovered on the local network but not yet paired.
#[derive(uniffi::Record, Clone)]
pub struct MobileAvailableDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
}

/// Single clipboard item surfaced through the bridge to the mobile app.
#[derive(uniffi::Record, Clone)]
pub struct MobileClipItem {
    pub id: String,
    pub content: String,
    pub source_device: String,
    pub received_at_ms: u64,
    pub kind: String,
    pub html: String,
    pub file_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: u64,
    pub file_expires_at_ms: u64,
    pub file_downloaded: bool,
    pub local_file_path: String,
}

fn device_type_name(value: DeviceType) -> String {
    match value {
        DeviceType::Windows => "windows",
        DeviceType::Android => "android",
        DeviceType::Linux => "linux",
        DeviceType::Macos => "macos",
        DeviceType::Ios => "ios",
        DeviceType::Unspecified => "unknown",
    }
    .to_string()
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("{reason}")]
    Message { reason: String },
}

impl From<String> for BridgeError {
    fn from(reason: String) -> Self {
        Self::Message { reason }
    }
}

impl From<&str> for BridgeError {
    fn from(reason: &str) -> Self {
        Self::Message {
            reason: reason.to_string(),
        }
    }
}

fn connection_name(value: ConnectionState) -> String {
    match value {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Unavailable => "unavailable",
    }
    .to_string()
}

/// The FFI bridge wrapper for the running core.
///
/// It owns the background task handles and exposes platform-neutral commands and
/// events to Android and iOS host applications.
#[derive(uniffi::Object)]
pub struct BridgeService {
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tasks: Vec<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    clipboard_cmd_tx: Option<
        tokio::sync::mpsc::UnboundedSender<clipx_core::clipboard::manager::ClipboardCommand>,
    >,
    device_tx: Option<tokio::sync::mpsc::UnboundedSender<DeviceCommands>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl BridgeService {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Inner::default()),
        })
    }

    pub async fn start(
        &self,
        data_dir: String,
        device_name: String,
        clipboard: Arc<dyn ClipboardPlatform>,
        notifier: Arc<dyn NotificationPlatform>,
        event: Arc<dyn ClipxEventListener>,
    ) {
        let mut inner = self.inner.lock().await;
        if inner.shutdown_tx.is_some() {
            return;
        }

        clipx_core::device::config::set_config_root(std::path::PathBuf::from(data_dir));
        clipx_core::device::config::set_device_name_override(device_name);

        let clipboard_adapter = ClipboardSinkAdapter(clipboard);
        let notifier_adapter = NotificationAdapter(notifier);
        let notification_engine = PlatformNotificationEngine::new(Arc::new(notifier_adapter));
        let core_event_listener: Arc<dyn clipx_core::platform::CoreEventListener> =
            Arc::new(crate::platform::ClipxEventAdapter(event));

        let (tasks, shutdown_tx, clipboard_cmd_tx, device_tx, _core_events_tx) =
            clipx_core::service::spawn_core_tasks(
                clipboard_adapter,
                notification_engine,
                Some(core_event_listener),
            );

        inner.tasks = tasks;
        inner.shutdown_tx = Some(shutdown_tx);
        inner.clipboard_cmd_tx = Some(clipboard_cmd_tx);
        inner.device_tx = Some(device_tx);
    }

    pub fn resolve_prompt(&self, prompt_id: String, result: NotifierDecision) {
        let result = match result {
            NotifierDecision::PairDecision(value) => {
                let decision = match value {
                    0 => clipx_core::notification::PairDecision::Allow,
                    1 => clipx_core::notification::PairDecision::Deny,
                    _ => clipx_core::notification::PairDecision::NoResponse,
                };
                PromptResult::Pair(decision)
            }
            NotifierDecision::IncomingClipboardDecision(value) => {
                let decision = if value == 0 {
                    clipx_core::notification::IncomingClipboardDecision::Copy
                } else {
                    clipx_core::notification::IncomingClipboardDecision::Ignore
                };
                PromptResult::Clipboard(decision)
            }
        };

        clipx_core::notification::platform::resolve_prompt(prompt_id, result);
    }

    pub async fn report_clipboard_changed(&self, content: String) {
        let tx = self.inner.lock().await.clipboard_cmd_tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(
                clipx_core::clipboard::manager::ClipboardCommand::LocalChangeDetected {
                    payload: clipx_core::clipboard::manager::ClipboardPayload::Text(content),
                },
            );
        }
    }

    async fn require_connected_device(&self) -> Result<(), BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::Connected { reply_to: reply_tx })
            .map_err(|_| BridgeError::from("core device manager is stopped"))?;
        let devices = reply_rx
            .await
            .map_err(|_| BridgeError::from("core device manager is stopped"))?;
        if devices.is_empty() {
            return Err(BridgeError::from("No connected devices"));
        }
        Ok(())
    }

    /// Explicit user-initiated clipboard send from the mobile UI.
    /// This bypasses local-change deduplication because pressing Send is
    /// deliberate, including when the text is identical to a previous send.
    pub async fn send_clipboard(&self, content: String) -> Result<(), BridgeError> {
        self.require_connected_device().await?;
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::SendLocal {
                payload: clipx_core::clipboard::manager::ClipboardPayload::Text(content),
            },
        )
        .map_err(|_| "core clipboard manager is stopped".to_string())
        .map_err(BridgeError::from)
    }

    pub async fn send_rich_text(&self, text: String, html: String) -> Result<(), BridgeError> {
        self.require_connected_device().await?;
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::SendLocal {
                payload: clipx_core::clipboard::manager::ClipboardPayload::RichText { text, html },
            },
        )
        .map_err(|_| BridgeError::from("core clipboard manager is stopped"))
    }

    pub async fn send_image(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<(), BridgeError> {
        self.require_connected_device().await?;
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::SendLocal {
                payload: clipx_core::clipboard::manager::ClipboardPayload::Image {
                    width,
                    height,
                    rgba,
                },
            },
        )
        .map_err(|_| BridgeError::from("core clipboard manager is stopped"))
    }

    pub async fn send_file(
        &self,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    ) -> Result<(), BridgeError> {
        self.require_connected_device().await?;
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::SendLocal {
                payload: clipx_core::clipboard::manager::ClipboardPayload::FileBytes {
                    name,
                    mime_type,
                    data,
                },
            },
        )
        .map_err(|_| BridgeError::from("core clipboard manager is stopped"))
    }

    /// Send a file by staging/copying its path in the core instead of loading the file into a byte array.
    pub async fn send_file_path(
        &self,
        name: String,
        mime_type: String,
        path: String,
    ) -> Result<(), BridgeError> {
        self.require_connected_device().await?;
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::SendFilePath {
                name,
                mime_type,
                path: std::path::PathBuf::from(path),
                reply_to: reply_tx,
            },
        )
        .map_err(|_| BridgeError::from("core clipboard manager is stopped"))?;
        reply_rx
            .await
            .map_err(|_| BridgeError::from("core clipboard manager is stopped"))?
            .map_err(BridgeError::from)
    }

    pub async fn download_clipboard_file(&self, id: String) -> Result<String, BridgeError> {
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::DownloadHistoryFile {
                entry_id: id,
                reply_to: reply_tx,
            },
        )
        .map_err(|_| BridgeError::from("core clipboard manager is stopped"))?;
        reply_rx
            .await
            .map_err(|_| BridgeError::from("core clipboard manager is stopped"))
    }

    pub async fn get_identity(&self) -> Result<MobileIdentity, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::GetIdentity { reply_to: reply_tx })
            .map_err(|_| "core device manager is stopped".to_string())?;
        let identity = reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())?;
        Ok(MobileIdentity {
            fingerprint: identity.device_id.clone(),
            id: identity.device_id,
            name: identity.device_name,
            device_type: device_type_name(identity.device_type),
            ip_address: identity.ip_addr,
            ws_port: identity.ws_port,
        })
    }

    pub async fn get_paired_devices(&self) -> Result<Vec<MobilePairedDevice>, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::GetPaired { reply_to: reply_tx })
            .map_err(|_| "core device manager is stopped".to_string())?;
        let devices = reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())?;
        devices
            .into_iter()
            .map(|device| {
                let connection = ConnectionState::try_from(device.connection)
                    .map_err(|_| format!("invalid connection state {}", device.connection))?;
                let device_type = DeviceType::try_from(device.device_type)
                    .map_err(|_| format!("invalid device type {}", device.device_type))?;
                Ok(MobilePairedDevice {
                    id: device.id,
                    name: device.name,
                    device_type: device_type_name(device_type),
                    connection: connection_name(connection),
                    ip_address: device.address,
                    ws_port: device.ws_port,
                    auto_connect: device.auto_connect,
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(BridgeError::from)
    }

    pub async fn get_available_devices(&self) -> Result<Vec<MobileAvailableDevice>, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::GetSeen {
            mode: SeenMode::Untrusted,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        let devices = reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())?;
        Ok(devices
            .into_iter()
            .map(|device| MobileAvailableDevice {
                id: device.id,
                name: device.name,
                device_type: device_type_name(device.device_type),
            })
            .collect())
    }

    pub async fn pair_device(&self, device_id: String) -> Result<String, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::Pair {
            device_id,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn connect_device(&self, device_id: String) -> Result<String, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::Connect {
            device_id,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn disconnect_device(&self, device_id: String) -> Result<String, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::Disconnect {
            device_id,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn set_auto_connect(
        &self,
        device_id: String,
        auto_connect: bool,
    ) -> Result<bool, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::SetAutoConnect {
            device_id,
            auto_connect,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn forget_device(&self, device_id: String) -> Result<String, BridgeError> {
        let tx = self.device_sender().await?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(DeviceCommands::ForgetDevice {
            device_id,
            reply_to: reply_tx,
        })
        .map_err(|_| "core device manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core device manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn get_clipboard_history(
        &self,
        limit: u32,
    ) -> Result<Vec<MobileClipItem>, BridgeError> {
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        let limit = if limit == 0 {
            None
        } else {
            Some(limit as usize)
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::GetHistory {
                limit,
                reply_to: reply_tx,
            },
        )
        .map_err(|_| "core clipboard manager is stopped".to_string())?;
        let items = reply_rx
            .await
            .map_err(|_| "core clipboard manager is stopped".to_string())
            .map_err(BridgeError::from)?;
        Ok(items
            .into_iter()
            .map(|item| MobileClipItem {
                id: item.id,
                content: item.content,
                source_device: item.source_device,
                received_at_ms: item.received_at_ms,
                kind: match item.kind {
                    clipx_core::clipboard::clipstore::ClipKind::Text => "text",
                    clipx_core::clipboard::clipstore::ClipKind::RichText => "rich_text",
                    clipx_core::clipboard::clipstore::ClipKind::Image => "image",
                    clipx_core::clipboard::clipstore::ClipKind::File => "file",
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
            .collect())
    }

    pub async fn remove_clipboard_entry(&self, id: String) -> Result<bool, BridgeError> {
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::RemoveEntry {
                id,
                reply_to: reply_tx,
            },
        )
        .map_err(|_| "core clipboard manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core clipboard manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn clear_clipboard_history(&self) -> Result<(), BridgeError> {
        let tx = self
            .inner
            .lock()
            .await
            .clipboard_cmd_tx
            .clone()
            .ok_or_else(|| "core clipboard manager is stopped".to_string())?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(
            clipx_core::clipboard::manager::ClipboardCommand::ClearHistory { reply_to: reply_tx },
        )
        .map_err(|_| "core clipboard manager is stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "core clipboard manager is stopped".to_string())
            .map_err(BridgeError::from)
    }

    pub async fn stop(&self) {
        let (shutdown_tx, tasks) = {
            let mut inner = self.inner.lock().await;
            let shutdown_tx = inner.shutdown_tx.take();
            inner.clipboard_cmd_tx = None;
            inner.device_tx = None;
            (shutdown_tx, std::mem::take(&mut inner.tasks))
        };

        if let Some(tx) = shutdown_tx {
            let _ = tx.send(true);
        }
        for task in tasks {
            let _ = task.await;
        }
    }
}

impl BridgeService {
    async fn device_sender(
        &self,
    ) -> Result<tokio::sync::mpsc::UnboundedSender<DeviceCommands>, String> {
        self.inner
            .lock()
            .await
            .device_tx
            .clone()
            .ok_or_else(|| "core device manager is stopped".to_string())
    }
}
