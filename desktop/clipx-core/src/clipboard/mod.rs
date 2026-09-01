pub mod clipstore;
pub mod manager;

// compiles for windows, linux and macos only
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub mod watcher;

/// this is here to handle running clipx-core barely on environment arboard is not supported
/// when clipx-core used as a library — the platform must implement the object that give core
/// access of the system clipboard in any kind of mode — pull(watch and poll periodically) 
/// or push(the platform pushes and core uses) modes
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub mod watcher {
    use tokio::sync::{mpsc, watch};

    pub async fn watch_clipboard(
        mut _shutdown_rx: watch::Receiver<bool>,
        _tx: mpsc::UnboundedSender<String>,
    ) {
        tracing::info!(
            "Watcher is not implemented for non-desktop Os — only implemented for Linux, Windows, Macos"
        )
    }
}
