use super::{platform::{NotificationEngine as Engine, NotificationFuture}, IncomingClipboardDecision, PairDecision, Prompt};
use std::{collections::HashMap, sync::{Arc, Mutex}};
use std::time::Duration;

use tauri_winrt_notification::{Progress, Toast};
use tokio::sync::oneshot;

// TODO: swap for ClipX's own registered AUMID once the installer creates a
// Start Menu shortcut with an AppUserModelID set. Borrowing PowerShell's id
// works for development but ships the toast labeled "Windows PowerShell".
const APP_ID: &str = if cfg!(debug_assertions) {Toast::POWERSHELL_APP_ID } else {"com.godiegh.clipx"};

#[derive(Clone)]
pub struct NotificationEngine {
    transfer_titles: Arc<Mutex<HashMap<String, String>>>,
}

impl NotificationEngine {
    pub fn new() -> Self {
        Self { transfer_titles: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Shared implementation for all interactive Windows notifications.
    /// The platform-specific toast mechanics live here exactly once.
    /// Individual notification types only provide their buttons,
    /// action mapping, and fallback decision.
    async fn show_prompt<D, F>(
        &self,
        title: &str,
        body: &str,
        timeout: Duration,
        buttons: &[(&str, &str)],
        map_action: F,
        fallback: D,
    ) -> D
    where
        D: Send + Clone + 'static,
        F: Fn(Option<&str>) -> D + Send + Sync + 'static,
    {
        let (tx, rx) = oneshot::channel::<D>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        let tx_activated = Arc::clone(&tx);
        let tx_dismissed = Arc::clone(&tx);

        let title = title.to_string();
        let body = body.to_string();

        let map_action = Arc::new(map_action);
        let map_action_activated = Arc::clone(&map_action);

        // .show() performs synchronous COM/WinRT work.
        // block_in_place prevents that work from blocking other Tokio tasks
        // on this worker thread.
        let result = tokio::task::block_in_place(|| {
            let mut toast = Toast::new(APP_ID)
                .title(&title)
                .text1(&body)
                .duration(tauri_winrt_notification::Duration::Long);

            for &(label, action) in buttons {
                toast = toast.add_button(label, action);
            }

            toast
                .on_activated(move |action| {
                    let decision = map_action_activated(action.as_deref());

                    if let Some(sender) =
                        tx_activated.lock().unwrap().take()
                    {
                        let _ = sender.send(decision);
                    }

                    Ok(())
                })
                .on_dismissed(move |_reason| {
                    if let Some(sender) =
                        tx_dismissed.lock().unwrap().take()
                    {
                        let _ = sender.send(fallback.clone());
                    }

                    Ok(())
                })
                .show()
        });

        if let Err(e) = result {
            tracing::error!("failed to show toast: {e:?}");
            return map_action(None);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => decision,

            // Sender disappeared without producing a decision.
            Ok(Err(_)) => map_action(None),

            // We stopped waiting. The toast may still be visible.
            Err(_) => map_action(None),
        }
    }

    fn transfer_notification(&self, file_id: String, done: u64, total: u64, state: String, message: String) {
        let title = {
            let mut titles = self.transfer_titles.lock().unwrap();
            if matches!(state.as_str(), "complete" | "failed" | "expired") {
                titles.remove(&file_id).unwrap_or_else(|| message.clone())
            } else {
                let title = message
                    .strip_prefix("Requesting ")
                    .or_else(|| message.strip_prefix("Receiving "))
                    .or_else(|| message.strip_prefix("Sending "))
                    .unwrap_or(&message)
                    .split(" · ")
                    .next()
                    .unwrap_or(&message)
                    .to_string();
                titles.entry(file_id.clone()).or_insert_with(|| title.clone()).clone()
            }
        };

        let tag = format!("clipx-transfer-{file_id}");
        let fraction = if total == 0 { 0.0 } else { (done as f64 / total as f64).clamp(0.0, 1.0) as f32 };
        let terminal = matches!(state.as_str(), "complete" | "failed" | "expired");
        let successful = state == "complete";

        if terminal {
            let toast = Toast::new(APP_ID)
                .title(&title)
                .text1(if successful { "Downloaded successfully" } else { &message });
            let result = if successful && std::path::Path::new(&message).is_file() {
                let path = message.clone();
                toast
                    .add_button("Show in folder", "open")
                    .on_activated(move |action| {
                        if action.as_deref() == Some("open") {
                            let _ = std::process::Command::new("explorer.exe")
                                .arg(format!("/select,{path}"))
                                .spawn();
                        }
                        Ok(())
                    })
                    .show()
            } else {
                toast.show()
            };
            if let Err(error) = result {
                tracing::error!(?error, "failed to show file transfer completion toast");
            }
            return;
        }

        let progress = Progress {
            tag,
            title,
            status: match state.as_str() {
                "requesting" => "Starting download…".to_string(),
                "receiving" => "Downloading…".to_string(),
                "sending" => "Sending…".to_string(),
                "saving" => "Saving…".to_string(),
                _ => message,
            },
            value: fraction,
            value_string: if total > 0 { format!("{fraction:.0}%") } else { String::new() },
        };

        let result = if done == 0 || state == "requesting" {
            Toast::new(APP_ID).progress(&progress).show()
        } else {
            Toast::new(APP_ID).set_progress(&progress).map(|_| ())
        };
        if let Err(error) = result {
            tracing::debug!(?error, "file transfer notification update failed");
            let _ = Toast::new(APP_ID).progress(&progress).show();
        }
    }
}

impl Engine for NotificationEngine {
    fn ask_pair<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            let (title, body) = prompt.render();

            self.show_prompt(
                &title,
                &body,
                timeout,
                &[("Allow", "allow"), ("Deny", "deny")],
                |action| match action {
                    Some("allow") => PairDecision::Allow,
                    Some("deny") => PairDecision::Deny,
                    _ => PairDecision::NoResponse,
                },
                PairDecision::NoResponse,
            )
            .await
        })
    }

    fn ask_clipboard<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision> {
        Box::pin(async move {
            let (title, body) = prompt.render();

            let action_label = match prompt {
                Prompt::IncomingClipboard { kind: super::IncomingClipboardKind::Image, .. } => "Save image",
                Prompt::IncomingClipboard { kind: super::IncomingClipboardKind::File, .. } => "Download file",
                Prompt::IncomingClipboard { .. } => "Copy to clipboard",
                _ => "Copy to clipboard",
            };

            self.show_prompt(
                &title,
                &body,
                timeout,
                &[(action_label, "copy")],
                |action| match action {
                    Some("copy") => IncomingClipboardDecision::Copy,
                    _ => IncomingClipboardDecision::Ignore,
                },
                IncomingClipboardDecision::Ignore,
            )
            .await
        })
    }

    fn notify_info<'a>(
        &'a self,
        title: &'a str,
        body: String,
    ) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            let title = title.to_string();

            tokio::task::block_in_place(|| {
                let _ = Toast::new(APP_ID)
                    .title(&title)
                    .text1(&body)
                    .show();
            });
        })
    }

    fn notify_file_transfer<'a>(
        &'a self,
        file_id: String,
        done: u64,
        total: u64,
        state: String,
        message: String,
    ) -> NotificationFuture<'a, ()> {
        let this = self.clone();
        Box::pin(async move {
            tokio::task::block_in_place(|| {
                this.transfer_notification(file_id, done, total, state, message);
            });
        })
    }
}

impl Default for NotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}