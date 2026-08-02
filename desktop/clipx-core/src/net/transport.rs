use tokio::sync::{watch, mpsc};

pub async fn cordinator(
    mut shutdown_rx: watch::Receiver<bool>,
    mut clipboard_rx: mpsc::UnboundedReceiver<String>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
            Some(value) = clipboard_rx.recv() => {
                tracing::info!("transport received: {value}");
                // later: wrap in ClipboardMessage, send to connected trusted devices
            }
        }
    }
    tracing::info!("transport coordinator stopped");
}