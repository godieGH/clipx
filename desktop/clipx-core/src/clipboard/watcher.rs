use arboard::Clipboard;
use tokio::{sync::{mpsc, watch}, time::{sleep, Duration}};

pub async fn watch_clipboard(
    mut shutdown_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<String>,
) {
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to init clipboard: {}", e);
            return;
        }
    };

    let mut last_content = clipboard.get_text().unwrap_or_default();
    let last_char_count = last_content.chars().count();
    let last_snippet: String = last_content.chars().take(50).collect();
    tracing::info!("clipboard watcher started initial content: {:?}{}", last_snippet, if last_char_count > 50 {"..."} else {""});

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = sleep(Duration::from_millis(500)) => {}
        }

        if *shutdown_rx.borrow() {
            break;
        }

        match clipboard.get_text() {
            Ok(current) if current != last_content => {
                let current_char_count = current.chars().count();
                let current_snippet: String = current.chars().take(50).collect();
                tracing::info!("Clipboard changed: {:?}{}", current_snippet, if current_char_count > 50 {"..."} else {""});
                last_content = current.clone();
                if tx.send(current).is_err() {
                    tracing::warn!("Clipboard event receiver dropped — transport might be down; continuing to watch");
                    // do not break; keep watching clipboard even if transport is down
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Clipboard read error: {e}"),
        }
    }

    tracing::info!("clipboard watcher stopped");
}