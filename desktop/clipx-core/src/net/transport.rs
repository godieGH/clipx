use tokio::sync::{watch, mpsc};
use tokio::time::{Duration, sleep};

pub async fn cordinator(
    mut shutdown_rx: watch::Receiver<bool>,
    mut clipboard_rx: mpsc::UnboundedReceiver<String>
) {

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = sleep(Duration::from_millis(500)) => {}
        }

        while let Some(value) = clipboard_rx.recv().await {
            println!("{value}");
        }
    }

    tracing::info!("Transport coordinate stopped");

}