use std::sync::Arc;
use clipx_core::device::identity;
use tokio::task::JoinHandle;

use crate::platform::{ClipboardPlatform, ClipxEventListener, NotificationPlatform};


/// The ffi bridge wrapper for the core service 
/// it holds the inner core machinery + state as one
#[derive(uniffi::Object)]
pub struct BridgeService {
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tasks: Vec<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl BridgeService {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: tokio::sync::Mutex::new(Inner::default())})
    }

    pub async fn start(
        &self,
        data_dir: String,
        _clipboard: Arc<dyn ClipboardPlatform>,
        _notifier: Arc<dyn NotificationPlatform>,
        _event: Arc<dyn ClipxEventListener>,
    ) {
        clipx_core::device::config::set_config_root(std::path::PathBuf::from(data_dir));

        // let (tasks, shutdown_tx) = clipx_core::service::spawn_core_tasks(
        //      clipboard_platform, notification_platform, event_listener
        //);
        // let mut _inner = self.inner.lock().await;
        // inner.tasks = tasks;
        // inner.shutdown_tx = Some(shutdown_tx);
    }

    /// this is called by android to turn/tell the core tasks that the host
    /// is going off so they have to wrap it up — we can use just direct abort
    /// since android shutdown might be abruptly but this is a try the best as you can to 
    /// clean the core or the host is not going to run any of your part
    /// maybe we can figure out later how to make not to close without proper cleanup time
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;

        if let Some(tx) = inner.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        for task in inner.tasks.drain(..) {
            // or maybe just hard abort call and not waiting 
            let _ = task.await;
        }
    }
}