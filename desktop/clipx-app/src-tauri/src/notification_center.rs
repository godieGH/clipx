//! The notification center: prompts, transfers and messages from the core,
//! shown in their own window instead of through the OS notification system.
//!
//! The list lives here, in the app's Rust process, not in the window: the window
//! can be closed or hidden and the list survives for as long as the app runs.
//! `nc_store.rs` holds the rules; this file connects them to the core (events in,
//! answers out) and to the window.

use crate::{
    nc_store::{Kind, Notification, PromptIn, Store, TransferIn, TransferView},
    AppState,
};
use clipx_lib::{clipx, message::IpcCmdBridge};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::MenuItem, AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Wry,
};

const WINDOW_LABEL: &str = "notifications";

/// Id of the tray icon, so the center can update its tooltip.
pub const TRAY_ID: &str = "clipx";

/// What a badge shows: how many notifications need the user or are still
/// running, and how many of those are prompts waiting for an answer.
#[derive(serde::Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    unread: usize,
    waiting: usize,
}

pub struct NotificationCenter {
    app: AppHandle,
    store: Mutex<Store>,
    emit_scheduled: AtomicBool,
    /// The tray's "Notifications" item, whose text carries the unread count.
    tray_item: Mutex<Option<MenuItem<Wry>>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl NotificationCenter {
    pub fn new(app: AppHandle) -> Arc<Self> {
        Arc::new(Self {
            app,
            store: Mutex::new(Store::new()),
            emit_scheduled: AtomicBool::new(false),
            tray_item: Mutex::new(None),
        })
    }

    /// Hands over the tray's "Notifications" item and shows the current count on it.
    pub fn attach_tray_item(&self, item: MenuItem<Wry>) {
        *self.tray_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(item);
        self.update_badge();
    }

    fn badge(&self) -> Badge {
        let store = self.store();
        Badge {
            unread: store.unread_count(),
            waiting: store.waiting_prompt_count(),
        }
    }

    /// Shows the counts on the tray (tooltip and menu text) and tells the main
    /// window, which puts them on its bell.
    fn update_badge(&self) {
        let badge = self.badge();
        let tooltip = match (badge.unread, badge.waiting) {
            (0, _) => "Clipx".to_string(),
            (n, 0) => format!("Clipx - {n} unread"),
            (n, w) => format!("Clipx - {n} unread, {w} waiting for you"),
        };
        if let Some(tray) = self.app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
        if let Some(item) = self
            .tray_item
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let text = if badge.unread > 0 {
                format!("Notifications ({})", badge.unread)
            } else {
                "Notifications".to_string()
            };
            let _ = item.set_text(text);
        }
        let _ = self.app.emit("nc-badge", badge);
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        // A panic while holding the lock must not take the whole list down.
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes an event from the core. Prompts, their endings and plain messages
    /// are only for this center; transfer events are also forwarded to the main
    /// window by the caller, so they are noted here and then left alone.
    pub fn handle_event(self: &Arc<Self>, event: &clipx::ipc_event::Event) {
        use clipx::ipc_event::Event;
        let now = now_ms();
        match event {
            Event::NotificationPrompt(p) => {
                use clipx::notification_prompt_event::{ContentKind, Kind as Wire};
                let kind = match Wire::try_from(p.kind) {
                    Ok(Wire::PairRequest) => Kind::PairRequest,
                    Ok(Wire::ConfirmCode) => Kind::ConfirmCode,
                    Ok(Wire::IncomingClipboard) => Kind::IncomingClipboard,
                    Err(_) => return,
                };
                let content_kind = match ContentKind::try_from(p.content_kind) {
                    Ok(ContentKind::RichText) => "richText",
                    Ok(ContentKind::Image) => "image",
                    Ok(ContentKind::File) => "file",
                    _ => "text",
                };
                self.store().on_prompt(
                    PromptIn {
                        prompt_id: p.prompt_id.clone(),
                        kind,
                        peer_name: p.peer_name.clone(),
                        code: p.code.clone(),
                        content: p.content.clone(),
                        content_kind: content_kind.into(),
                        timeout_ms: p.timeout_ms,
                    },
                    now,
                );
                self.changed();
                // Something is waiting for an answer: make sure the user can see it.
                self.show_if_hidden();
            }
            Event::NotificationClosed(c) => {
                use clipx::notification_closed_event::Reason;
                let timed_out = matches!(Reason::try_from(c.reason), Ok(Reason::TimedOut));
                self.store().on_prompt_closed(&c.prompt_id, timed_out, now);
                self.changed();
            }
            Event::NotificationInfo(i) => {
                self.store()
                    .on_info(i.id.clone(), i.title.clone(), i.body.clone(), now);
                self.changed();
            }
            Event::FileTransfer(t) => {
                self.store().on_transfer(transfer_in(t), now);
                self.changed();
            }
            _ => {}
        }
    }

    /// Tells the window the list changed. Progress events arrive far faster than
    /// anyone can read them, so changes within a short moment make one signal;
    /// the window then asks for the list.
    fn changed(self: &Arc<Self>) {
        if self.emit_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            this.emit_scheduled.store(false, Ordering::Release);
            this.update_badge();
            let _ = this.app.emit("nc-changed", ());
        });
    }

    fn show_if_hidden(self: &Arc<Self>) {
        let visible = self
            .app
            .get_webview_window(WINDOW_LABEL)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        if !visible {
            if let Err(error) = show_center(&self.app) {
                eprintln!("ClipX: could not show the notification center: {error}");
            }
        }
    }

    fn transfer(&self, id: &str) -> Option<TransferView> {
        self.store().get(id).and_then(|n| n.transfer.clone())
    }
}

fn transfer_in(t: &clipx::FileTransferEvent) -> TransferIn {
    TransferIn {
        device_id: t.device_id.clone(),
        device_name: t.device_name.clone(),
        file_id: t.file_id.clone(),
        file_name: t.file_name.clone(),
        direction: t.direction.clone(),
        done: t.done,
        total: t.total,
        state: t.state.clone(),
        message: t.message.clone(),
        can_pause: t.can_pause,
    }
}

/// Gap kept to the screen edges, the same one the main window opens with.
const EDGE_MARGIN: i32 = 30;

/// Laid over the main window, the center is this much wider on each side and
/// starts this much lower (logical pixels), so the main window's top stays
/// visible above it and both can be seen at once.
const OVERLAP_GROW: f64 = 60.0;
const OVERLAP_DROP: f64 = 150.0;

/// Opens the notification center window, or brings the existing one forward.
/// Closing it only hides it (the app-wide close handler), so its list stays.
///
/// It is made like the main window: borderless and transparent (the page draws
/// the title bar and the Mica backdrop), opened over the main window when that
/// is showing, otherwise in the corner the main window opens in. After that the
/// user can move it and it stays where they left it.
pub fn show_center(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.unminimize();
        window.show()?;
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App("index.html".into()))
        .title("Clipx notifications")
        .inner_size(400.0, 700.0)
        .min_inner_size(400.0, 400.0)
        .decorations(false)
        .transparent(true)
        .resizable(true)
        .visible(false)
        .build()?;
    #[cfg(windows)]
    if let Err(error) = crate::configure_windows_context_menu(&window) {
        eprintln!("ClipX: could not set up the notification window's context menu: {error}");
    }
    place_like_main(app, &window);
    window.show()?;
    keep_inside_work_area(&window);
    let _ = window.set_focus();
    Ok(())
}

/// Lays `window` across the lower part of the main window when that is showing:
/// wider than it, bottom edges aligned, top lower, so the main window's top is
/// still visible. Otherwise puts it in the bottom-right corner of the work area,
/// where the main window opens. Best effort: if the system refuses (some Wayland
/// setups) the default placement stays.
fn place_like_main(app: &AppHandle, window: &tauri::WebviewWindow) {
    if let Some(main) = app.get_webview_window("main") {
        let showing = main.is_visible().unwrap_or(false) && !main.is_minimized().unwrap_or(false);
        if showing {
            if let (Ok(pos), Ok(size)) = (main.outer_position(), main.outer_size()) {
                let scale = main.scale_factor().unwrap_or(1.0);
                let grow = (OVERLAP_GROW * scale) as i32;
                let drop = (OVERLAP_DROP * scale) as i32;
                let min_height = (360.0 * scale) as i32;

                let mut width = size.width as i32 + 2 * grow;
                let mut height = (size.height as i32 - drop).max(min_height);
                let mut x = pos.x - grow;
                let mut y = pos.y + size.height as i32 - height;

                // Stay on the screen the main window is on.
                if let Ok(Some(monitor)) = main.current_monitor() {
                    let area = monitor.work_area();
                    let margin = (EDGE_MARGIN as f64 * scale) as i32;
                    let (left, top) = (area.position.x + margin, area.position.y + margin);
                    let right = area.position.x + area.size.width as i32 - margin;
                    let bottom = area.position.y + area.size.height as i32 - margin;
                    width = width.min(right - left);
                    height = height.min(bottom - top);
                    x = x.clamp(left, right - width);
                    y = y.clamp(top, bottom - height);
                }
                let _ = window.set_size(tauri::PhysicalSize::new(width as u32, height as u32));
                let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                return;
            }
        }
    }
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let area = monitor.work_area();
    let x = area.position.x + area.size.width as i32 - size.width as i32 - EDGE_MARGIN;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - EDGE_MARGIN;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Pulls `window` back inside the work area, keeping `EDGE_MARGIN` to every
/// edge. It reads the window's real position and size, so it also corrects
/// anything the system changed after the placement above.
fn keep_inside_work_area(window: &tauri::WebviewWindow) {
    let (Ok(Some(monitor)), Ok(pos), Ok(size)) = (
        window.current_monitor(),
        window.outer_position(),
        window.outer_size(),
    ) else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let margin = (EDGE_MARGIN as f64 * scale) as i32;
    let area = monitor.work_area();
    let left = area.position.x + margin;
    let top = area.position.y + margin;
    let right = area.position.x + area.size.width as i32 - margin;
    let bottom = area.position.y + area.size.height as i32 - margin;
    let x = pos.x.clamp(left, (right - size.width as i32).max(left));
    let y = pos.y.clamp(top, (bottom - size.height as i32).max(top));
    if (x, y) != (pos.x, pos.y) {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
pub async fn open_notification_center(app: AppHandle) -> Result<(), String> {
    show_center(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn nc_list(nc: State<'_, Arc<NotificationCenter>>) -> Vec<Notification> {
    nc.store().snapshot()
}

/// The counts a badge shows; later changes arrive as the `nc-badge` event.
#[tauri::command]
pub fn nc_badge(nc: State<'_, Arc<NotificationCenter>>) -> Badge {
    nc.badge()
}

/// Answers a waiting prompt: `allow` / `deny` for pairing and code prompts,
/// `copy` / `ignore` for incoming clipboard content (`copy` also downloads a file).
#[tauri::command]
pub async fn nc_resolve(
    state: State<'_, AppState>,
    nc: State<'_, Arc<NotificationCenter>>,
    id: String,
    decision: String,
) -> Result<(), String> {
    use clipx::resolve_prompt_request::Decision;
    let (wire, outcome) = match decision.as_str() {
        "allow" => (Decision::Allow, "allowed"),
        "deny" => (Decision::Deny, "denied"),
        "copy" => (Decision::Copy, "copied"),
        "ignore" => (Decision::Ignore, "ignored"),
        _ => return Err("Unknown answer".into()),
    };
    let result = state
        .ipc
        .lock()
        .await
        .resolve_prompt(id.clone(), wire)
        .await;
    match result {
        Ok(()) => {
            nc.store().on_prompt_answered(&id, outcome, now_ms());
            nc.changed();
            Ok(())
        }
        Err(message) => {
            // Too late (answered elsewhere or timed out): show it as over.
            if message.contains("already answered") {
                nc.store().on_prompt_closed(&id, true, now_ms());
                nc.changed();
            }
            Err(message)
        }
    }
}

/// `pause`, `resume` or `cancel` on one transfer notification (by its id).
/// A paused download is resumed by asking for the file again; a paused upload
/// asks the receiver to continue. A refusal (an older peer that cannot pause)
/// comes back as the error text.
#[tauri::command]
pub async fn nc_transfer_action(
    state: State<'_, AppState>,
    nc: State<'_, Arc<NotificationCenter>>,
    id: String,
    action: String,
) -> Result<(), String> {
    use clipx::control_transfer_request::Action;
    let Some(t) = nc.transfer(&id) else {
        return Err("That transfer is no longer listed".into());
    };
    let device = Some(t.device_id.clone());
    let mut ipc = state.ipc.lock().await;
    let result = match action.as_str() {
        "pause" => {
            ipc.control_transfer(t.file_id.clone(), device, Action::Pause)
                .await
        }
        "cancel" => {
            ipc.control_transfer(t.file_id.clone(), device, Action::Cancel)
                .await
        }
        "resume" if t.direction == "send" => {
            ipc.control_transfer(t.file_id.clone(), device, Action::Resume)
                .await
        }
        "resume" => ipc.download_clipboard_file(t.file_id.clone()).await,
        _ => Err("Unknown action".into()),
    };
    result.map(|_| ())
}

#[tauri::command]
pub fn nc_mark_read(nc: State<'_, Arc<NotificationCenter>>, id: String) -> bool {
    let changed = nc.store().mark_read(&id, now_ms());
    if changed {
        nc.changed();
    }
    changed
}

#[tauri::command]
pub fn nc_mark_all_read(nc: State<'_, Arc<NotificationCenter>>) -> usize {
    let changed = nc.store().mark_all_read(now_ms());
    if changed > 0 {
        nc.changed();
    }
    changed
}

#[tauri::command]
pub fn nc_clear_read(nc: State<'_, Arc<NotificationCenter>>) -> usize {
    let removed = nc.store().clear_read();
    if removed > 0 {
        nc.changed();
    }
    removed
}

/// Drops one read notification from the list.
#[tauri::command]
pub fn nc_remove(nc: State<'_, Arc<NotificationCenter>>, id: String) -> bool {
    let removed = nc.store().remove(&id);
    if removed {
        nc.changed();
    }
    removed
}

/// Opens the folder of a finished download with the file selected. The saved
/// path is not part of the transfer event, so it is looked up in the core's
/// clipboard history by the transfer's file id.
#[tauri::command]
pub async fn nc_reveal_download(
    state: State<'_, AppState>,
    nc: State<'_, Arc<NotificationCenter>>,
    id: String,
) -> Result<(), String> {
    let Some(t) = nc.transfer(&id) else {
        return Err("That transfer is no longer listed".into());
    };
    if t.direction == "send" || t.state != "complete" {
        return Err("There is no saved file for this transfer".into());
    }
    let history = state.ipc.lock().await.get_clipboard_history(None).await?;
    let path = history
        .into_iter()
        .find(|e| e.file_id.as_deref() == Some(t.file_id.as_str()))
        .and_then(|e| e.local_file_path)
        .ok_or("Could not find where this file was saved")?;
    crate::reveal_file_location(path)
}

/// Adds transfers that started before the app attached (a restarted app, or a
/// transfer begun while the window was never opened). Live ones keep their state.
#[tauri::command]
pub async fn nc_refresh_transfers(
    state: State<'_, AppState>,
    nc: State<'_, Arc<NotificationCenter>>,
) -> Result<(), String> {
    let transfers = state.ipc.lock().await.list_transfers().await?;
    let rows = transfers
        .into_iter()
        .map(|t| TransferIn {
            device_id: t.device_id,
            device_name: t.device_name,
            file_id: t.file_id,
            file_name: t.file_name,
            direction: t.direction,
            done: t.done,
            total: t.total,
            state: t.state,
            message: String::new(),
            can_pause: t.can_pause,
        })
        .collect();
    nc.store().seed_transfers(rows, now_ms());
    nc.changed();
    Ok(())
}
