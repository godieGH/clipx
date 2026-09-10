use super::{
    IncomingClipboardDecision, PairDecision, Prompt,
    platform::{NotificationEngine as Engine, NotificationFuture},
};
use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, mpsc},
};

use tauri_winrt_notification::{Progress, Toast};
use tokio::sync::oneshot;
use windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{
        NotificationData, NotificationUpdateResult, ToastNotification, ToastNotificationManager,
    },
};

// TODO: swap for ClipX's own registered AUMID once the installer creates a
// Start Menu shortcut with an AppUserModelID set. Borrowing PowerShell's id
// works for development but ships the toast labeled "Windows PowerShell".
const APP_ID: &str = if cfg!(debug_assertions) {
    Toast::POWERSHELL_APP_ID
} else {
    "com.godiegh.clipx"
};

#[derive(Debug)]
struct TransferNotification {
    file_id: String,
    file_name: String,
    direction: String,
    done: u64,
    total: u64,
    state: String,
    message: String,
}

#[derive(Clone)]
pub struct NotificationEngine {
    transfer_tx: mpsc::Sender<TransferNotification>,
}

impl NotificationEngine {
    pub fn new() -> Self {
        let (transfer_tx, transfer_rx) = mpsc::channel::<TransferNotification>();
        // WinRT toast callbacks and the progress-update calls below want to
        // run off the async runtime, and doing them from one dedicated
        // thread (fed by a channel) means progress events from concurrent
        // transfer tasks get applied to the shell in the order they were
        // produced instead of racing each other across Tokio worker threads.
        std::thread::Builder::new()
            .name("clipx-windows-transfer-notifications".into())
            .spawn(move || {
                let mut titles: HashMap<String, String> = HashMap::new();
                // file_ids for which we've already called Toast::show() —
                // every update after that goes through set_progress_always
                // instead, since re-showing would pop a new toast banner.
                let mut shown: HashSet<String> = HashSet::new();
                let mut sequence_numbers: HashMap<String, u32> = HashMap::new();
                let mut last_done: HashMap<String, u64> = HashMap::new();
                let mut finished: HashSet<String> = HashSet::new();
                while let Ok(event) = transfer_rx.recv() {
                    Self::handle_transfer_notification(
                        &mut titles,
                        &mut shown,
                        &mut sequence_numbers,
                        &mut last_done,
                        &mut finished,
                        event,
                    );
                }
            })
            .expect("failed to start Windows transfer notification thread");

        Self { transfer_tx }
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
            let toast = Toast::new(APP_ID)
                .title(&title)
                .text1(&body)
                .duration(tauri_winrt_notification::Duration::Long);

            let mut toast = toast;
            for &(label, action) in buttons {
                toast = toast.add_button(label, action);
            }

            toast
                .on_activated(move |action| {
                    let decision = map_action_activated(action.as_deref());

                    if let Some(sender) = tx_activated.lock().unwrap().take() {
                        let _ = sender.send(decision);
                    }

                    Ok(())
                })
                .on_dismissed(move |_reason| {
                    if let Some(sender) = tx_dismissed.lock().unwrap().take() {
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

    fn handle_transfer_notification(
        titles: &mut HashMap<String, String>,
        shown: &mut HashSet<String>,
        sequence_numbers: &mut HashMap<String, u32>,
        last_done: &mut HashMap<String, u64>,
        finished: &mut HashSet<String>,
        event: TransferNotification,
    ) {
        let TransferNotification {
            file_id,
            file_name,
            direction,
            done,
            total,
            state,
            message,
        } = event;
        let terminal = matches!(
            state.as_str(),
            "complete" | "failed" | "expired" | "cancelled" | "interrupted"
        );

        if !terminal {
            // A terminal event wins permanently. Progress events come from
            // independent async transfer tasks, so a late/out-of-order one
            // must never resurrect a completed/failed notification, and an
            // older byte offset must never move the progress bar backwards.
            if finished.contains(&file_id) {
                return;
            }
            if let Some(previous) = last_done.get(&file_id) {
                if done < *previous {
                    return;
                }
            }
            last_done.insert(file_id.clone(), done);
        }

        if terminal {
            finished.insert(file_id.clone());
        }

        let title = if terminal {
            titles.remove(&file_id).unwrap_or_else(|| {
                if file_name.is_empty() {
                    message.clone()
                } else {
                    file_name.clone()
                }
            })
        } else {
            titles
                .entry(file_id.clone())
                .or_insert_with(|| {
                    if !file_name.is_empty() {
                        file_name.clone()
                    } else {
                        message
                            .strip_prefix("Requesting ")
                            .or_else(|| message.strip_prefix("Receiving "))
                            .or_else(|| message.strip_prefix("Sending "))
                            .unwrap_or(&message)
                            .split(" · ")
                            .next()
                            .unwrap_or(&message)
                            .to_string()
                    }
                })
                .clone()
        };

        let tag = format!("clipx-transfer-{file_id}");

        if terminal {
            // Make the final progress state explicit before showing the
            // completion popup. This is important for small/fast transfers
            // where the last receiving event may already represent 100%.
            if state == "complete" && total > 0 && shown.contains(&file_id) {
                let final_progress = Progress {
                    tag: tag.clone(),
                    title: title.clone(),
                    status: if direction == "send" {
                        "Sending finished".to_string()
                    } else {
                        "Download finished".to_string()
                    },
                    value: 1.0,
                    value_string: "100%".to_string(),
                };
                let sequence = sequence_numbers
                    .entry(file_id.clone())
                    .and_modify(|n| *n = n.saturating_add(1))
                    .or_insert(2);
                match set_progress_always(APP_ID, &final_progress, *sequence) {
                    Ok(NotificationUpdateResult::Succeeded) => {
                        // Give Windows a brief chance to paint 100% before the
                        // completion popup replaces the progress notification.
                        std::thread::sleep(Duration::from_millis(120));
                    }
                    Ok(result) => {
                        tracing::debug!(?result, %file_id, "final Windows progress update did not apply");
                    }
                    Err(error) => {
                        tracing::debug!(?error, %file_id, "final Windows progress update failed");
                    }
                }
            }

            shown.remove(&file_id);
            sequence_numbers.remove(&file_id);
            last_done.remove(&file_id);

            let completion_text = if state == "complete" {
                if direction == "send" {
                    "Sending finished"
                } else {
                    "Download finished"
                }
            } else {
                &message
            };

            let mut toast = Toast::new(APP_ID).title(&title).text1(completion_text);

            if state == "complete"
                && direction == "download"
                && std::path::Path::new(&message).is_file()
            {
                let path = message.clone();
                let open_path = path.clone();
                toast = toast
                    .add_button("Open", "open")
                    .add_button("Show in folder", "reveal")
                    .on_activated(move |action| {
                        match action.as_deref() {
                            Some("open") => {
                                let opened = std::process::Command::new("explorer.exe")
                                    .arg(&open_path)
                                    .spawn()
                                    .is_ok();
                                if !opened {
                                    let _ = reveal_path_in_explorer(std::path::Path::new(&path));
                                }
                            }
                            Some("reveal") => {
                                let _ = reveal_path_in_explorer(std::path::Path::new(&path));
                            }
                            _ => {}
                        }
                        Ok(())
                    });
            }

            if let Err(error) = toast.show() {
                tracing::error!(?error, "failed to show file transfer completion toast");
            }
            return;
        }

        let fraction = if total == 0 {
            0.0
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0) as f32
        };
        let progress = Progress {
            tag,
            title: title.clone(),
            status: match state.as_str() {
                "requesting" => {
                    if direction == "send" {
                        "Sending file".to_string()
                    } else {
                        "Downloading file".to_string()
                    }
                }
                "receiving" => "Downloading…".to_string(),
                "sending" => {
                    if done == 0 {
                        "Sending file".to_string()
                    } else {
                        "Sending…".to_string()
                    }
                }
                "saving" => "Saving…".to_string(),
                _ => message.clone(),
            },
            value: fraction,
            value_string: if total > 0 {
                format!("{fraction:.0}%")
            } else {
                String::new()
            },
        };

        if !shown.contains(&file_id) {
            match show_progress_toast(APP_ID, &progress) {
                Ok(()) => {
                    // First notification uses sequence 1. Subsequent updates
                    // must use strictly increasing sequence numbers.
                    shown.insert(file_id.clone());
                    sequence_numbers.insert(file_id.clone(), 1);
                }
                Err(error) => tracing::error!(
                    ?error,
                    "failed to show Windows progress toast"
                ),
            }
            return;
        }

        let sequence = sequence_numbers
            .entry(file_id.clone())
            .and_modify(|n| *n = n.saturating_add(1))
            .or_insert(2);

        match set_progress_always(APP_ID, &progress, *sequence) {
            Ok(NotificationUpdateResult::Succeeded) => {}
            Ok(NotificationUpdateResult::NotificationNotFound) => {
                // The user may have dismissed the progress notification.
                // Recreate it at the current percentage and restart the
                // sequence for the new notification instance.
                match show_progress_toast(APP_ID, &progress) {
                    Ok(()) => {
                        shown.insert(file_id.clone());
                        sequence_numbers.insert(file_id.clone(), 1);
                    }
                    Err(error) => tracing::error!(
                        ?error,
                        "failed to re-show Windows progress toast after dismissal"
                    ),
                }
            }
            Ok(result) => {
                tracing::warn!(?result, %file_id, "Windows progress notification was not updated");
            }
            Err(error) => {
                tracing::warn!(?error, %file_id, "Windows progress notification update failed");
            }
        }
    }
}

/// Sends a progress-only WinRT toast update with the explicit sequence
/// number for this transfer. Windows requires later updates to use a larger
/// sequence number than the previous update for the same tag.
fn set_progress_always(
    app_id: &str,
    progress: &Progress,
    sequence_number: u32,
) -> windows::core::Result<NotificationUpdateResult> {
    use windows::{
        Foundation::Collections::StringMap,
        UI::Notifications::{NotificationData, ToastNotificationManager},
        core::HSTRING,
    };

    // Field-for-field the same as tauri_winrt_notification::Toast::set_progress's
    // own implementation (same StringMap keys, same call sequence) — the
    // only change is the caller-provided sequence number below.
    let map = StringMap::new()?;
    map.Insert(
        &HSTRING::from("progressTitle"),
        &HSTRING::from(&progress.title),
    )?;
    map.Insert(
        &HSTRING::from("progressStatus"),
        &HSTRING::from(&progress.status),
    )?;
    map.Insert(
        &HSTRING::from("progressValue"),
        &HSTRING::from(&progress.value.to_string()),
    )?;
    map.Insert(
        &HSTRING::from("progressValueString"),
        &HSTRING::from(&progress.value_string),
    )?;

    let data = NotificationData::CreateNotificationDataWithValuesAndSequenceNumber(&map, sequence_number)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&app_id.to_string()))?;
    notifier.UpdateWithTag(&data, &HSTRING::from(&progress.tag))
}

// new function, next to set_progress_always:
fn show_progress_toast(app_id: &str, progress: &Progress) -> windows::core::Result<()> {
    let xml = windows::core::HSTRING::from(format!(
        r#"<toast><visual><binding template="ToastGeneric">
            <text>{}</text>
            <progress title="{{progressTitle}}" value="{{progressValue}}" valueStringOverride="{{progressValueString}}" status="{{progressStatus}}" />
        </binding></visual></toast>"#,
        progress.title
    ));

    let document = XmlDocument::new()?;
    document.LoadXml(&xml)?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.SetTag(&windows::core::HSTRING::from(&progress.tag))?;

    let data = NotificationData::new()?;
    data.Values()?.Insert(
        &windows::core::HSTRING::from("progressTitle"),
        &windows::core::HSTRING::from(&progress.title),
    )?;
    data.Values()?.Insert(
        &windows::core::HSTRING::from("progressValue"),
        &windows::core::HSTRING::from(progress.value.to_string()),
    )?;
    data.Values()?.Insert(
        &windows::core::HSTRING::from("progressValueString"),
        &windows::core::HSTRING::from(&progress.value_string),
    )?;
    data.Values()?.Insert(
        &windows::core::HSTRING::from("progressStatus"),
        &windows::core::HSTRING::from(&progress.status),
    )?;
    data.SetSequenceNumber(1)?;
    toast.SetData(&data)?;

    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&windows::core::HSTRING::from(app_id))?;
    notifier.Show(&toast)
}

#[cfg(windows)]
fn reveal_path_in_explorer(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;

    // Explorer's /select,<path> syntax is parsed as a single command-line
    // construct. Command::arg() may quote the whole argument when the path
    // contains spaces, which makes Explorer fall back to its default folder.
    // raw_arg preserves the required /select, prefix while still letting
    // Explorer receive the real path.
    let path = path.to_string_lossy();
    std::process::Command::new("explorer.exe")
        .raw_arg(format!(r#"/select,"{}""#, path))
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn reveal_path_in_explorer(_path: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows Explorer is unavailable on this platform",
    ))
}

impl Engine for NotificationEngine {
    // desktop/clipx-core/src/notification/windows_engine.rs

    fn ask_pair<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            let labels: &[(&str, &str)] = match prompt {
                Prompt::ConfirmCode { .. } => &[("Confirm", "allow"), ("Deny", "deny")],
                _ => &[("Allow", "allow"), ("Deny", "deny")],
            };
            let (title, body) = prompt.render();

            self.show_prompt(
                &title,
                &body,
                timeout,
                labels,
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
                Prompt::IncomingClipboard {
                    kind: super::IncomingClipboardKind::Image,
                    ..
                } => "Save image",
                Prompt::IncomingClipboard {
                    kind: super::IncomingClipboardKind::File,
                    ..
                } => "Download file",
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

    fn notify_info<'a>(&'a self, title: &'a str, body: String) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            let title = title.to_string();

            tokio::task::block_in_place(|| {
                let _ = Toast::new(APP_ID).title(&title).text1(&body).show();
            });
        })
    }

    fn notify_file_transfer<'a>(
        &'a self,
        file_id: String,
        file_name: String,
        direction: String,
        done: u64,
        total: u64,
        state: String,
        message: String,
    ) -> NotificationFuture<'a, ()> {
        let _ = self.transfer_tx.send(TransferNotification {
            file_id,
            file_name,
            direction,
            done,
            total,
            state,
            message,
        });
        // The notification thread owns the non-Send Toast/COM state.
        // Sending through a channel also preserves the order transfer
        // events were produced in instead of racing concurrent async tasks
        // against each other on the Tokio thread pool.
        Box::pin(async {})
    }
}

impl Default for NotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}
