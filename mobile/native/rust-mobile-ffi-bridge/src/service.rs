use std::sync::Arc;

use clipx_core::notification::platform::{PlatformNotificationEngine, PromptResult};
use tokio::task::JoinHandle;

use crate::platform::{
    ClipboardPlatform, ClipxEventAdapter, ClipxEventListener, ClipboardSinkAdapter,
    NotificationAdapter, NotificationPlatform, NotifierDecision,
};

/// The FFI bridge wrapper for the core service. It owns the running core
/// tasks and the channels Android uses to report host-side events.
#[derive(uniffi::Object)]
pub struct BridgeService {
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tasks: Vec<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    clipboard_cmd_tx: Option<tokio::sync::mpsc::UnboundedSender<clipx_core::clipboard::manager::ClipboardCommand>>,
    device_tx: Option<tokio::sync::mpsc::UnboundedSender<clipx_core::device::manager::DeviceCommands>>,
    core_events_tx: Option<tokio::sync::broadcast::Sender<clipx_core::platform::PushEvents>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl BridgeService {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: tokio::sync::Mutex::new(Inner::default()) })
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
        let event_adapter = Arc::new(ClipxEventAdapter(event));

        let (tasks, shutdown_tx, clipboard_cmd_tx, device_tx, events_tx) =
            clipx_core::service::spawn_core_tasks(
                clipboard_adapter,
                notification_engine,
                Some(event_adapter),
            );

        inner.tasks = tasks;
        inner.shutdown_tx = Some(shutdown_tx);
        inner.clipboard_cmd_tx = Some(clipboard_cmd_tx);
        inner.device_tx = Some(device_tx);
        inner.core_events_tx = Some(events_tx);
    }

    /// Resolves an interactive prompt created by the core notification engine.
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

    /// Reports a change detected by Android's system clipboard listener.
    pub async fn report_clipboard_changed(&self, content: String) {
        let tx = self.inner.lock().await.clipboard_cmd_tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(clipx_core::clipboard::manager::ClipboardCommand::LocalChangeDetected { content });
        }
    }

    /// Stops all core tasks and waits for them to finish.
    pub async fn stop(&self) {
        let (shutdown_tx, tasks) = {
            let mut inner = self.inner.lock().await;
            let shutdown_tx = inner.shutdown_tx.take();
            inner.clipboard_cmd_tx = None;
            inner.device_tx = None;
            inner.core_events_tx = None;
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
