// built for windows, linux and macos only
#[cfg(not(target_os = "android"))]
pub mod watcher;

#[cfg(target_os = "android")]
pub mod watcher {
    pub async fn watch_clipboard(
        mut shutdown_rx: watch::Receiver<bool>,
        tx: mpsc::UnboundedSender<String>,
    ) {
        // A stub if this is build for android os which arboard is not functional
        // For testing on termux environments
        tracing::info!("Watcher is not implemented for Android Os")
    }
}
