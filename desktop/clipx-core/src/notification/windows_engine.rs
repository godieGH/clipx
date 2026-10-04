// ClipX Windows notification engine
//
// Behavior:
//   - Clipboard previews are retained but compacted/truncated.
//   - Download progress has a short grace period so fast downloads never
//     flash a progress toast.
//   - Slow downloads get a progress toast even when the file itself is tiny.
//   - Zero/unknown-sized downloads never display 0/0 progress.
//   - Download progress never displays 100%; completion owns the final state.
//   - A download that has received all bytes waits for the explicit "complete"
//     event, which carries the real saved path for the completion toast.
//   - Progress toast updates are throttled, and only terminal events are
//     awaited by callers, so a fast sender never floods the notification thread.
//   - A progress toast the user dismissed is not re-created on later updates.
//   - Progress and completion notifications use separate tags.

use super::{
    IncomingClipboardDecision, NotificationConfig, PairDecision, ProgressText, Prompt,
    TransferNotificationOverride,
    platform::{NotificationEngine as Engine, NotificationFuture},
};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use tauri_winrt_notification::{Progress, Toast};
use tokio::sync::oneshot;

use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{NotificationData, NotificationUpdateResult, ToastNotificationManager},
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    core::{HSTRING, IInspectable, Interface},
};

const APP_ID: &str = if cfg!(debug_assertions) {
    Toast::POWERSHELL_APP_ID
} else {
    "com.godiegh.clipx"
};

// A progress toast is delayed briefly. Fast transfers only produce the final
// completion notification; slow transfers get normal progress feedback.
const PROGRESS_GRACE_PERIOD: Duration = Duration::from_millis(300);

const PROGRESS_TOAST_GROUP: &str = "clipx-transfer-progress";

// How long the finished "100% / Complete" state stays visible.
const FINAL_PROGRESS_HOLD: Duration = Duration::from_millis(2000);

// Keep useful clipboard preview text, but never send an entire clipboard
// payload into a Windows toast.
const MAX_NOTIFICATION_PREVIEW_CHARS: usize = 240;

struct TransferNotification {
    file_id: String,
    file_name: String,
    direction: String,
    done: u64,
    total: u64,
    state: String,
    message: String,
    notification_title: String,
    progress_text: ProgressText,
    completion: Option<oneshot::Sender<()>>,
}

struct NotificationWorker {
    tx: Option<mpsc::Sender<TransferNotification>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for NotificationWorker {
    fn drop(&mut self) {
        self.tx.take();

        if let Some(thread) = self.thread.take()
            && thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
    }
}

#[derive(Clone)]
pub struct NotificationEngine {
    worker: Arc<NotificationWorker>,
    transfer_config: Arc<NotificationConfig>,
}

impl NotificationEngine {
    pub fn new() -> Self {
        Self::with_config(NotificationConfig::default())
    }

    pub fn with_config(config: NotificationConfig) -> Self {
        let (transfer_tx, transfer_rx) = mpsc::channel::<TransferNotification>();

        let thread = thread::Builder::new()
            .name("clipx-windows-notification-host".into())
            .spawn(move || run_notification_host(transfer_rx))
            .expect("failed to start Windows notification thread");

        Self {
            worker: Arc::new(NotificationWorker {
                tx: Some(transfer_tx),
                thread: Some(thread),
            }),
            transfer_config: Arc::new(config),
        }
    }

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
        let body = notification_preview(body);

        let map_action = Arc::new(map_action);
        let map_action_activated = Arc::clone(&map_action);

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

        if let Err(error) = result {
            tracing::error!(?error, "failed to show toast");
            return map_action(None);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => map_action(None),
            Err(_) => {
                tracing::debug!("notification prompt timed out while toast may still be visible");
                map_action(None)
            }
        }
    }
}

fn run_notification_host(transfer_rx: mpsc::Receiver<TransferNotification>) {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

    if hr.0 < 0 {
        tracing::error!(
            hresult = hr.0,
            "failed to initialize Windows COM MTA for notification host"
        );
        return;
    }

    tracing::debug!("Windows notification host initialized as COM MTA");

    let mut states: HashMap<String, FileToastState> = HashMap::new();

    loop {
        let wait = next_progress_deadline(&states);

        let result = match wait {
            Some(duration) => transfer_rx.recv_timeout(duration),
            None => match transfer_rx.recv() {
                Ok(event) => Ok(event),
                Err(_) => Err(mpsc::RecvTimeoutError::Disconnected),
            },
        };

        match result {
            Ok(event) => handle_transfer_notification(&mut states, event),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                fire_due_progress_notifications(&mut states);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                fire_due_progress_notifications(&mut states);
                break;
            }
        }
    }

    unsafe {
        CoUninitialize();
    }

    tracing::debug!("Windows notification host stopped");
}

fn download_finished(pending: &PendingProgress) -> bool {
    !is_upload_direction(&pending.direction) && pending.done >= pending.total
}

fn next_progress_deadline(states: &HashMap<String, FileToastState>) -> Option<Duration> {
    let now = Instant::now();

    states
        .values()
        .filter_map(|state| {
            let pending = state.pending_progress.as_ref()?;

            if !pending.progress_shown {
                if pending.total == 0 || download_finished(pending) {
                    return None;
                }
                let deadline = state.started_at + PROGRESS_GRACE_PERIOD;
                return Some(deadline.saturating_duration_since(now));
            }

            if state.dirty && matches!(state.phase, ToastPhase::Shown { .. }) {
                let deadline = state.last_toast_update + MIN_TOAST_UPDATE_INTERVAL;
                return Some(deadline.saturating_duration_since(now));
            }

            None
        })
        .min()
}

fn fire_due_progress_notifications(states: &mut HashMap<String, FileToastState>) {
    let now = Instant::now();
    let mut show_ids = Vec::new();
    let mut flush_ids = Vec::new();

    for (file_id, state) in states.iter() {
        let Some(pending) = state.pending_progress.as_ref() else {
            continue;
        };

        if !pending.progress_shown {
            if pending.total == 0 || download_finished(pending) {
                continue;
            }
            if now >= state.started_at + PROGRESS_GRACE_PERIOD {
                show_ids.push(file_id.clone());
            }
        } else if state.dirty
            && matches!(state.phase, ToastPhase::Shown { .. })
            && now >= state.last_toast_update + MIN_TOAST_UPDATE_INTERVAL
        {
            flush_ids.push(file_id.clone());
        }
    }

    for id in show_ids {
        show_due_progress(states, &id);
    }
    for id in flush_ids {
        update_existing_progress(states, &id, false);
    }
}

fn show_due_progress(states: &mut HashMap<String, FileToastState>, file_id: &str) {
    let Some(existing) = states.get(file_id) else {
        return;
    };

    let Some(pending) = existing.pending_progress.as_ref() else {
        return;
    };

    if pending.progress_shown || matches!(existing.phase, ToastPhase::Done) {
        return;
    }

    let progress_tag = progress_tag(file_id);

    let Some(mut progress) = build_progress(
        &existing.title,
        &pending.direction,
        pending.done,
        pending.total,
        &pending.state,
        &pending.message,
        &pending.progress_text,
        &progress_tag,
    ) else {
        return;
    };

    // Ensure the first visible toast is never an already-finished download.
    if !is_upload_direction(&pending.direction) && pending.done >= pending.total {
        return;
    }

    progress.tag = progress_tag;

    match show_progress_toast(APP_ID, &pending.notification_title, &progress) {
        Ok(()) => {
            if let Some(state) = states.get_mut(file_id) {
                state.phase = ToastPhase::Shown { sequence: 1 };
                state.last_toast_update = Instant::now();

                if let Some(pending) = state.pending_progress.as_mut() {
                    pending.progress_shown = true;
                }
            }
        }
        Err(error) => {
            tracing::error!(
                ?error,
                %file_id,
                "failed to show delayed Windows progress toast"
            );
        }
    }
}

fn handle_transfer_notification(
    states: &mut HashMap<String, FileToastState>,
    mut event: TransferNotification,
) {
    let completion = event.completion.take();

    let TransferNotification {
        file_id,
        file_name,
        direction,
        done,
        total,
        state,
        message,
        notification_title,
        progress_text,
        completion: _,
    } = event;

    let result = handle_transfer_notification_inner(
        states,
        &file_id,
        &file_name,
        &direction,
        done,
        total,
        &state,
        &message,
        &notification_title,
        &progress_text,
    );

    if let Err(error) = result {
        tracing::error!(
            ?error,
            %file_id,
            "failed to process Windows transfer notification"
        );
    }

    if let Some(sender) = completion {
        let _ = sender.send(());
    }
}

fn handle_transfer_notification_inner(
    states: &mut HashMap<String, FileToastState>,
    file_id: &str,
    file_name: &str,
    direction: &str,
    done: u64,
    total: u64,
    state: &str,
    message: &str,
    notification_title: &str,
    progress_text: &ProgressText,
) -> windows::core::Result<()> {
    let is_upload = is_upload_direction(direction);
    let is_download = !is_upload;

    let done_forever = matches!(state, "complete" | "failed" | "expired");
    let resettable_terminal = matches!(state, "cancelled" | "interrupted");
    let terminal = done_forever || resettable_terminal;

    if matches!(
        states.get(file_id).map(|state| &state.phase),
        Some(ToastPhase::Done)
    ) {
        // The same file id can be downloaded again later; a new "requesting"
        // starts a fresh lifecycle, anything else is a stale event.
        if state == "requesting" {
            states.remove(file_id);
        } else {
            return Ok(());
        }
    }

    if !terminal
        && let Some(existing) = states.get(file_id)
            && done < existing.last_done {
                return Ok(());
            }

    let title = states
        .get(file_id)
        .map(|state| state.title.clone())
        .unwrap_or_else(|| derive_title(file_name, message));

    let progress_tag = progress_tag(file_id);
    let completion_tag = completion_tag(file_id);

    // ------------------------------------------------------------------------
    // Explicit terminal state
    // ------------------------------------------------------------------------

    if terminal {
        return handle_terminal_notification(
            states,
            file_id,
            &title,
            &progress_tag,
            &completion_tag,
            direction,
            done,
            total,
            state,
            message,
            notification_title,
            progress_text,
        );
    }

    // ------------------------------------------------------------------------
    // First active event
    // ------------------------------------------------------------------------

    if !states.contains_key(file_id) {
        states.insert(
            file_id.to_string(),
            FileToastState {
                title: title.clone(),
                last_done: done,
                started_at: Instant::now(),
                last_toast_update: Instant::now(),
                phase: ToastPhase::Pending,
                pending_progress: Some(PendingProgress {
                    direction: direction.to_string(),
                    done,
                    total,
                    state: state.to_string(),
                    message: message.to_string(),
                    notification_title: notification_title.to_string(),
                    progress_text: progress_text.clone(),
                    progress_shown: false,
                }),
                dirty: false,
            },
        );

        // Uploads retain the old immediate-progress behavior.
        // Downloads get the grace period.
        if is_upload {
            show_due_progress(states, file_id);
        }

        // A zero-sized download simply waits for the terminal event.
        // It does not create a 0/0 progress notification.
        return Ok(());
    }

    // ------------------------------------------------------------------------
    // Existing active lifecycle
    // ------------------------------------------------------------------------

    let progress_already_shown = matches!(
        states.get(file_id).map(|state| &state.phase),
        Some(ToastPhase::Shown { .. })
    );

    if let Some(existing) = states.get_mut(file_id) {
        existing.last_done = done;

        if let Some(pending) = existing.pending_progress.as_mut() {
            pending.direction = direction.to_string();
            pending.done = done;
            pending.total = total;
            pending.state = state.to_string();
            pending.message = message.to_string();
            pending.notification_title = notification_title.to_string();
            pending.progress_text = progress_text.clone();
        }
    }

    // ------------------------------------------------------------------------
    // Download lifecycle rules
    // ------------------------------------------------------------------------

    if is_download {
        // Never create a 0/0 progress toast.
        if total == 0 {
            return Ok(());
        }

        let still_in_grace = states
            .get(file_id)
            .map(|state| Instant::now() < state.started_at + PROGRESS_GRACE_PERIOD)
            .unwrap_or(false);

        if !progress_already_shown && still_in_grace {
            // If the transfer reaches an explicit terminal event during this
            // period, the terminal branch above wins and the progress toast
            // is never shown.
            return Ok(());
        }

        if done >= total {
            // All bytes are here. Show that on the toast (the "saving" stage is
            // forced past the throttle) instead of freezing at the last percentage.
            // The explicit "complete" event still ends the toast.
            if progress_already_shown {
                update_existing_progress(states, file_id, state == "saving");
            }
            return Ok(());
        }

        if !progress_already_shown {
            show_due_progress(states, file_id);
            return Ok(());
        }
    }

    // ------------------------------------------------------------------------
    // Existing progress update
    // ------------------------------------------------------------------------

    if progress_already_shown {
        update_existing_progress(states, file_id, false);
    }

    Ok(())
}

fn handle_terminal_notification(
    states: &mut HashMap<String, FileToastState>,
    file_id: &str,
    title: &str,
    progress_tag: &str,
    completion_tag: &str,
    direction: &str,
    done: u64,
    _total: u64,
    state: &str,
    message: &str,
    notification_title: &str,
    _progress_text: &ProgressText,
) -> windows::core::Result<()> {
    tracing::info!(%file_id, %state, %direction, %message, "terminal notification");

    let previous = states.remove(file_id);

    let previous_sequence = match previous.as_ref().map(|state| &state.phase) {
        Some(ToastPhase::Shown { sequence }) => Some(*sequence),
        _ => None,
    };

    if matches!(state, "complete" | "failed" | "expired") {
        states.insert(
            file_id.to_string(),
            FileToastState {
                title: title.to_string(),
                last_done: done,
                started_at: Instant::now(),
                last_toast_update: Instant::now(),
                phase: ToastPhase::Done,
                pending_progress: None,
                dirty: false,
            },
        );
    }

    match state {
        "complete" if !is_upload_direction(direction) => {
            if let Some(sequence) = previous_sequence {
                finish_progress_toast(
                    file_id,
                    title,
                    progress_tag,
                    sequence,
                    _progress_text,
                    _total,
                );
            }

            show_completion_toast(APP_ID, completion_tag, notification_title, title, message)?;
        }

        "complete" => {
            if let Some(sequence) = previous_sequence {
                finish_progress_toast(
                    file_id,
                    title,
                    progress_tag,
                    sequence,
                    _progress_text,
                    _total,
                );
            }

            let toast = Toast::new(APP_ID)
                .title(notification_title)
                .text1(&format!("{title} — Sending finished"));

            toast.show().map_err(|error| {
                tracing::error!(?error, %file_id, "failed to show upload completion toast");

                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    format!("failed to show upload completion toast: {error}"),
                )
            })?;
        }

        _ => {
            if previous_sequence.is_some()
                && let Err(error) = remove_toast_history(APP_ID, progress_tag) {
                    tracing::debug!(
                        ?error,
                        %file_id,
                        "could not remove previous progress toast"
                    );
                }

            let toast_message = notification_preview(message);

            let toast = Toast::new(APP_ID)
                .title(notification_title)
                .text1(&toast_message);

            if let Err(error) = toast.show() {
                tracing::error!(
                    ?error,
                    %file_id,
                    "failed to show terminal file transfer toast"
                );
            }
        }
    }

    Ok(())
}

fn update_existing_progress(
    states: &mut HashMap<String, FileToastState>,
    file_id: &str,
    force: bool,
) {
    // Throttle, but never lose the newest values: remember that one is
    // waiting (dirty) and let the host loop send it when the window ends.
    {
        let Some(s) = states.get_mut(file_id) else {
            return;
        };
        if !matches!(s.phase, ToastPhase::Shown { .. }) {
            return;
        }
        if !force && s.last_toast_update.elapsed() < MIN_TOAST_UPDATE_INTERVAL {
            s.dirty = true;
            return;
        }
        s.dirty = false;
    }

    let Some(state) = states.get(file_id) else {
        return;
    };
    let Some(pending) = state.pending_progress.as_ref() else {
        return;
    };
    let sequence = match state.phase {
        ToastPhase::Shown { sequence } => sequence,
        _ => return,
    };

    let progress_tag = progress_tag(file_id);

    let Some(progress) = build_progress(
        &state.title,
        &pending.direction,
        pending.done,
        pending.total,
        &pending.state,
        &pending.message,
        &pending.progress_text,
        &progress_tag,
    ) else {
        return;
    };

    let next_sequence = sequence.saturating_add(1);

    match set_progress_always(APP_ID, &progress, next_sequence) {
        Ok(NotificationUpdateResult::Succeeded) => {
            if let Some(state) = states.get_mut(file_id) {
                state.phase = ToastPhase::Shown {
                    sequence: next_sequence,
                };
                state.last_toast_update = Instant::now();
            }
        }
        Ok(NotificationUpdateResult::NotificationNotFound) => {
            if let Some(state) = states.get_mut(file_id) {
                state.phase = ToastPhase::Dismissed;
            }
        }
        Ok(result) => tracing::warn!(?result, %file_id, "progress update was not applied"),
        Err(error) => tracing::warn!(?error, %file_id, "progress update failed"),
    }
}

fn build_progress(
    title: &str,
    direction: &str,
    done: u64,
    total: u64,
    state: &str,
    message: &str,
    progress_text: &ProgressText,
    tag: &str,
) -> Option<Progress> {
    if total == 0 {
        return None;
    }

    let is_upload = is_upload_direction(direction);

    let mut fraction = (done as f64 / total as f64).clamp(0.0, 1.0) as f32;

    if !is_upload {
        fraction = fraction.min(0.999);
    }

    let status = match state {
        "requesting" => {
            if is_upload {
                "Sending file"
            } else {
                "Downloading file"
            }
        }
        "receiving" => "Downloading…",
        "sending" => {
            if done == 0 {
                "Sending file"
            } else {
                "Sending…"
            }
        }
        "saving" => "Saving…",
        _ => message,
    }
    .to_string();

    Some(Progress {
        tag: tag.to_string(),
        title: title.to_string(),
        status,
        value: fraction,
        value_string: format_progress_text(progress_text, done, total, fraction),
    })
}

enum ToastPhase {
    Pending,
    Shown {
        sequence: u32,
    },
    /// The user dismissed the progress toast; never bring it back.
    Dismissed,
    Done,
}

struct PendingProgress {
    direction: String,
    done: u64,
    total: u64,
    state: String,
    message: String,
    notification_title: String,
    progress_text: ProgressText,
    progress_shown: bool,
}

struct FileToastState {
    title: String,
    last_done: u64,
    started_at: Instant,
    last_toast_update: Instant,
    phase: ToastPhase,
    pending_progress: Option<PendingProgress>,
    dirty: bool,
}

// Minimum gap between two updates of the same progress toast.
const MIN_TOAST_UPDATE_INTERVAL: Duration = Duration::from_millis(300);

fn progress_tag(file_id: &str) -> String {
    format!("clipx-transfer-progress-{file_id}")
}

fn completion_tag(file_id: &str) -> String {
    format!("clipx-transfer-complete-{file_id}")
}

fn is_upload_direction(direction: &str) -> bool {
    matches!(
        direction.to_ascii_lowercase().as_str(),
        "send" | "sending" | "upload" | "uploading"
    )
}

fn derive_title(file_name: &str, message: &str) -> String {
    if !file_name.is_empty() {
        return file_name.to_string();
    }

    message
        .strip_prefix("Requesting ")
        .or_else(|| message.strip_prefix("Receiving "))
        .or_else(|| message.strip_prefix("Sending "))
        .unwrap_or(message)
        .split(" · ")
        .next()
        .unwrap_or(message)
        .to_string()
}

fn notification_preview(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut chars = compact.chars();

    let preview: String = chars
        .by_ref()
        .take(MAX_NOTIFICATION_PREVIEW_CHARS)
        .collect();

    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn set_progress_always(
    app_id: &str,
    progress: &Progress,
    sequence_number: u32,
) -> windows::core::Result<NotificationUpdateResult> {
    use windows::Foundation::Collections::StringMap;

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
        &HSTRING::from(progress.value.to_string()),
    )?;

    map.Insert(
        &HSTRING::from("progressValueString"),
        &HSTRING::from(&progress.value_string),
    )?;

    let data =
        NotificationData::CreateNotificationDataWithValuesAndSequenceNumber(&map, sequence_number)?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;

    notifier.UpdateWithTagAndGroup(
        &data,
        &HSTRING::from(&progress.tag),
        &HSTRING::from(PROGRESS_TOAST_GROUP),
    )
}

fn show_progress_toast(
    app_id: &str,
    notification_title: &str,
    progress: &Progress,
) -> windows::core::Result<()> {
    let notification_title = escape_xml_text(notification_title);

    let xml = HSTRING::from(format!(
        r#"<toast duration="long">
        <visual>
            <binding template="ToastGeneric">
                <text>{notification_title}</text>
                <progress
                    title="{{progressTitle}}"
                    value="{{progressValue}}"
                    valueStringOverride="{{progressValueString}}"
                    status="{{progressStatus}}" />
            </binding>
        </visual>
    </toast>"#
    ));

    let document = XmlDocument::new()?;
    document.LoadXml(&xml)?;

    let toast = windows::UI::Notifications::ToastNotification::CreateToastNotification(&document)?;

    toast.SetTag(&HSTRING::from(&progress.tag))?;
    toast.SetGroup(&HSTRING::from(PROGRESS_TOAST_GROUP))?;

    let data = NotificationData::new()?;

    data.Values()?.Insert(
        &HSTRING::from("progressTitle"),
        &HSTRING::from(&progress.title),
    )?;

    data.Values()?.Insert(
        &HSTRING::from("progressValue"),
        &HSTRING::from(progress.value.to_string()),
    )?;

    data.Values()?.Insert(
        &HSTRING::from("progressValueString"),
        &HSTRING::from(&progress.value_string),
    )?;

    data.Values()?.Insert(
        &HSTRING::from("progressStatus"),
        &HSTRING::from(&progress.status),
    )?;

    data.SetSequenceNumber(1)?;
    toast.SetData(&data)?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;

    notifier.Show(&toast)
}

thread_local! {
    static ACTIVE_COMPLETION_TOASTS:
        std::cell::RefCell<
            HashMap<
                String,
                (
                    Instant,
                    windows::UI::Notifications::ToastNotification,
                ),
            >
        > = std::cell::RefCell::new(HashMap::new());
}

fn keep_completion_toast_alive(
    tag: &str,
    notification: windows::UI::Notifications::ToastNotification,
) {
    ACTIVE_COMPLETION_TOASTS.with(|toasts| {
        let mut toasts = toasts.borrow_mut();

        toasts.insert(tag.to_string(), (Instant::now(), notification));

        toasts.retain(|_, (created, _)| created.elapsed() < Duration::from_secs(6 * 60 * 60));
    });
}

fn show_completion_toast(
    app_id: &str,
    tag: &str,
    notification_title: &str,
    file_name: &str,
    file_path: &str,
) -> windows::core::Result<()> {
    use windows::UI::Notifications::ToastActivatedEventArgs;

    let document = XmlDocument::new()?;

    let toast = document.CreateElement(&HSTRING::from("toast"))?;
    let visual = document.CreateElement(&HSTRING::from("visual"))?;
    let binding = document.CreateElement(&HSTRING::from("binding"))?;

    binding.SetAttribute(&HSTRING::from("template"), &HSTRING::from("ToastGeneric"))?;

    let title = document.CreateElement(&HSTRING::from("text"))?;
    title.SetInnerText(&HSTRING::from(notification_title))?;
    binding.AppendChild(&title)?;

    let status = document.CreateElement(&HSTRING::from("text"))?;
    status.SetInnerText(&HSTRING::from("Download finished"))?;
    binding.AppendChild(&status)?;

    let file = document.CreateElement(&HSTRING::from("text"))?;
    file.SetInnerText(&HSTRING::from(file_name))?;
    binding.AppendChild(&file)?;

    visual.AppendChild(&binding)?;
    toast.AppendChild(&visual)?;

    let actions = document.CreateElement(&HSTRING::from("actions"))?;
    let action = document.CreateElement(&HSTRING::from("action"))?;

    action.SetAttribute(
        &HSTRING::from("content"),
        &HSTRING::from("Open file location"),
    )?;

    action.SetAttribute(
        &HSTRING::from("arguments"),
        &HSTRING::from("clipx:show-file-location"),
    )?;

    actions.AppendChild(&action)?;
    toast.AppendChild(&actions)?;
    document.AppendChild(&toast)?;

    let notification =
        windows::UI::Notifications::ToastNotification::CreateToastNotification(&document)?;

    notification.SetTag(&HSTRING::from(tag))?;

    let file_path = std::path::PathBuf::from(file_path);

    let handler: TypedEventHandler<windows::UI::Notifications::ToastNotification, IInspectable> =
        TypedEventHandler::new(move |_sender, args| {
            let args: &Option<IInspectable> = &args;

            let Some(args) = args.as_ref() else {
                return Ok(());
            };

            let Ok(args) = args.cast::<ToastActivatedEventArgs>() else {
                return Ok(());
            };

            let Ok(arguments) = args.Arguments() else {
                return Ok(());
            };

            if arguments == "clipx:show-file-location" {
                let path = file_path.clone();

                let _ = thread::Builder::new()
                    .name("clipx-open-file-location".into())
                    .spawn(move || {
                        if let Err(error) = reveal_path_in_explorer(&path) {
                            tracing::warn!(
                                ?error,
                                path = ?path,
                                "failed to reveal downloaded file in Explorer"
                            );
                        }
                    });
            }

            Ok(())
        });

    notification.Activated(&handler)?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;

    notifier.Show(&notification)?;

    keep_completion_toast_alive(tag, notification);

    Ok(())
}

fn finish_progress_toast(
    file_id: &str,
    title: &str,
    progress_tag: &str,
    sequence: u32,
    progress_text: &ProgressText,
    total: u64,
) {
    let final_progress = Progress {
        tag: progress_tag.to_string(),
        title: title.to_string(),
        status: "Complete".to_string(),
        value: 1.0,
        value_string: format_progress_text(progress_text, total, total, 1.0),
    };

    match set_progress_always(APP_ID, &final_progress, sequence.saturating_add(1)) {
        Ok(NotificationUpdateResult::Succeeded) => thread::sleep(FINAL_PROGRESS_HOLD),
        Ok(result) => tracing::warn!(?result, %file_id, "final progress state not applied"),
        Err(error) => tracing::warn!(?error, %file_id, "failed to show final progress state"),
    }

    if let Err(error) = remove_toast_history(APP_ID, progress_tag) {
        tracing::warn!(?error, %file_id, "could not remove previous progress toast");
    }
}

fn remove_toast_history(app_id: &str, tag: &str) -> windows::core::Result<()> {
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(
        &HSTRING::from(tag),
        &HSTRING::from(PROGRESS_TOAST_GROUP),
        &HSTRING::from(app_id),
    )
}

fn format_progress_text(mode: &ProgressText, done: u64, total: u64, fraction: f32) -> String {
    match mode {
        ProgressText::Percentage => {
            if total == 0 {
                String::new()
            } else {
                format!("{:.0}%", fraction * 100.0)
            }
        }

        ProgressText::Bytes => {
            if total == 0 {
                format_bytes(done)
            } else {
                format!("{} / {}", format_bytes(done), format_bytes(total))
            }
        }

        ProgressText::Custom(text) => text.clone(),
        ProgressText::Hidden => String::new(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];

    let mut value = bytes as f64;
    let mut unit = 0usize;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(windows)]
fn reveal_path_in_explorer(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;

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
                if let Err(error) = Toast::new(APP_ID).title(&title).text1(&body).show() {
                    tracing::warn!(?error, "failed to show information toast");
                }
            });
        })
    }

    fn notify_file_transfer(
        &self,
        file_id: String,
        file_name: String,
        direction: String,
        done: u64,
        total: u64,
        state: String,
        message: String,
        override_config: Option<TransferNotificationOverride>,
    ) -> NotificationFuture<'static, ()> {
        tracing::info!(
            %file_id, %file_name, %direction, done, total, %state, %message,
            "notify_file_transfer event"
        );

        let transfer_title = override_config
            .as_ref()
            .and_then(|config| config.transfer_title.clone())
            .unwrap_or_else(|| self.transfer_config.transfer_title.clone());

        let progress_text = override_config
            .and_then(|config| config.progress_text)
            .unwrap_or_else(|| self.transfer_config.progress_text.clone());

        // Only terminal events are awaited by callers. Progress events are
        // fire-and-forget so a fast sender is never slowed down (or able to
        // flood the notification thread with acknowledgements).
        let terminal = matches!(
            state.as_str(),
            "complete" | "failed" | "expired" | "cancelled" | "interrupted"
        );

        let (completion, wait) = if terminal {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let event = TransferNotification {
            file_id,
            file_name,
            direction,
            done,
            total,
            state,
            message,
            notification_title: transfer_title,
            progress_text,
            completion,
        };

        let Some(sender) = self.worker.tx.as_ref() else {
            tracing::warn!("Windows notification worker sender is unavailable");

            return Box::pin(async {});
        };

        if sender.send(event).is_err() {
            tracing::warn!("Windows notification worker has stopped");

            return Box::pin(async {});
        }

        Box::pin(async move {
            if let Some(rx) = wait {
                let _ = rx.await;
            }
        })
    }
}

impl Default for NotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}
