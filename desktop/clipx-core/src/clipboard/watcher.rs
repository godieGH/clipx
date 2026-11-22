use super::manager::ClipboardPayload;
use arboard::Clipboard;
use percent_encoding::percent_decode_str;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use tokio::{
    sync::{mpsc, watch},
    time::{Duration, sleep},
};

pub async fn watch_clipboard(
    mut shutdown_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<ClipboardPayload>,
) {
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to init clipboard: {e}");
            return;
        }
    };
    tracing::info!("clipboard watcher started");

    let mut last = String::new();
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
            _ = sleep(Duration::from_millis(500)) => {}
        }
        if *shutdown_rx.borrow() {
            break;
        }

        if let Ok(list) = clipboard.get().file_list()
            && !list.is_empty()
        {
            let paths: Vec<String> = list
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();

            let marker = format!("file:{}", paths.join("|"));
            if marker != last {
                last = marker;
                tracing::debug!(
                    files = paths.len(),
                    "clipboard file selection change detected"
                );
                let _ = tx.send(ClipboardPayload::Files(paths));
            }
            continue;
        }

        match clipboard.get_image() {
            Ok(image) => {
                let mut hasher = DefaultHasher::new();
                image.width.hash(&mut hasher);
                image.height.hash(&mut hasher);
                image.bytes.hash(&mut hasher);
                let marker = format!(
                    "image:{}x{}:{:x}",
                    image.width,
                    image.height,
                    hasher.finish()
                );
                if marker != last {
                    last = marker;
                    tracing::debug!(
                        width = image.width,
                        height = image.height,
                        bytes = image.bytes.len(),
                        "clipboard image change detected"
                    );
                    let _ = tx.send(ClipboardPayload::Image {
                        width: image.width as u32,
                        height: image.height as u32,
                        rgba: image.bytes.to_vec(),
                    });
                    continue;
                }
            }
            Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => tracing::debug!(%error, "clipboard image read failed"),
        }

        if let Ok(html) = clipboard.get().html()
            && !html.trim().is_empty()
        {
            let text = clipboard.get_text().unwrap_or_default();
            let mut hasher = DefaultHasher::new();
            html.hash(&mut hasher);
            text.hash(&mut hasher);
            let marker = format!("rich:{:x}", hasher.finish());
            if marker != last {
                last = marker;
                tracing::debug!(
                    text_bytes = text.len(),
                    html_bytes = html.len(),
                    "clipboard rich-text change detected"
                );
                let _ = tx.send(ClipboardPayload::RichText { text, html });
            }
            continue;
        }

        match clipboard.get_text() {
            Ok(current) if current != last => {
                last = current.clone();
                // Some desktop file managers expose copied file selections as
                // newline-delimited file:// URIs in the text clipboard. Treat
                // those as file offers rather than pushing file bytes through
                // the normal clipboard path.
                if let Some(paths) = parse_file_uris(&current)
                    && !paths.is_empty()
                {
                    tracing::debug!(files = paths.len(), "clipboard file URI change detected");
                    let _ = tx.send(ClipboardPayload::Files(paths));
                    continue;
                }
                tracing::debug!(bytes = current.len(), "clipboard text change detected");
                let _ = tx.send(ClipboardPayload::Text(current));
            }
            Ok(_) => {}
            Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => tracing::debug!(%error, "clipboard text read failed"),
        }
    }
    tracing::info!("clipboard watcher stopped");
}

fn parse_file_uris(text: &str) -> Option<Vec<String>> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() || !lines.iter().all(|s| s.starts_with("file://")) {
        return None;
    }
    let paths = lines
        .into_iter()
        .filter_map(|uri| {
            let rest = uri.strip_prefix("file://")?;
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            if !rest.starts_with('/') {
                return None;
            }
            percent_decode_str(rest)
                .decode_utf8()
                .ok()
                .map(|p| p.into_owned())
        })
        .collect::<Vec<_>>();
    if paths.is_empty() { None } else { Some(paths) }
}
