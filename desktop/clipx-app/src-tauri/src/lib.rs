mod error_dialog;

use clipx_lib::{clipx, ipc, message::types, message::IpcCmdBridge};
#[cfg(windows)]
use windows::core::Interface;
#[cfg(windows)]
use webview2_com::{take_pwstr, ContextMenuRequestedEventHandler, Microsoft::Web::WebView2::Win32::{ICoreWebView2_11}};
use tauri::{
    async_runtime::Mutex,
    menu::{Menu, MenuItem},
    tray::{self, MouseButton, MouseButtonState},
    Emitter, Manager, State,
};

pub struct AppState {
    ipc: Mutex<ipc::non_blocking::Client>,
}

fn mime_from_name(name: &str) -> String {
    match std::path::Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp", "pdf" => "application/pdf", "txt" => "text/plain", "html" | "htm" => "text/html", _ => "application/octet-stream",
    }.to_string()
}

#[cfg(windows)]
fn configure_windows_context_menu(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| unsafe {
        let core = match webview.controller().CoreWebView2() {
            Ok(value) => value,
            Err(error) => { eprintln!("ClipX: unable to access WebView2: {error}"); return; }
        };
        let core11: ICoreWebView2_11 = match core.cast() {
            Ok(value) => value,
            Err(error) => { eprintln!("ClipX: WebView2 context-menu API unavailable: {error}"); return; }
        };
        let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()); };
            let items = args.MenuItems()?;
            let mut count = 0u32;
            items.Count(&mut count)?;
            let mut remove = Vec::new();
            for i in 0..count {
                let item = items.GetValueAtIndex(i)? ;
                let mut name = windows::core::PWSTR::null();
                item.Name(&mut name)?;
                if matches!(take_pwstr(name).as_str(), "saveAs" | "print" | "moreTools") { remove.push(i); }
            }
            for i in remove.into_iter().rev() {  items.RemoveValueAtIndex(i)?; }
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(error) = core11.add_ContextMenuRequested(&handler, &mut token) {
            eprintln!("ClipX: unable to install WebView2 context-menu filter: {error}");
        }
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut client = ipc::non_blocking::Client::new("clipx");
    if let Err(e) = tauri::async_runtime::block_on(client.start()) {
        #[cfg(windows)]
        error_dialog::show_core_error(&e);

        #[cfg(not(windows))]
        eprintln!("Failed to start IPC: {e}");
        return;
    };

    let events_rx = client.take_events();

    let state = AppState {
        ipc: Mutex::new(client),
    };

    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            // Forward every push from the core onto the webview as a plain
            // Tauri event. The frontend swaps its setInterval polling for
            // `listen("devices-changed" | "clipboard-changed", ...)` and
            // re-fetches on demand — same invoke commands as before, just
            // triggered by a push instead of a timer.
            if let Some(mut events_rx) = events_rx {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(event) = events_rx.recv().await {
                        match event.event {
                            Some(clipx::ipc_event::Event::DevicesChanged(_)) => {
                                let _ = app_handle.emit("devices-changed", ());
                            }
                            Some(clipx::ipc_event::Event::ClipboardChanged(_)) => {
                                let _ = app_handle.emit("clipboard-changed", ());
                            }
                            Some(clipx::ipc_event::Event::PairingEvent(payload)) => {
                                let _ = app_handle.emit("pairing-event", payload);
                            }
                            Some(clipx::ipc_event::Event::FileTransfer(payload)) => {
                                let _ = app_handle.emit("file-transfer", payload);
                            }
                            None => continue,
                        }
                    }
                });
            }

            #[cfg(windows)]
            if let Some(main_window) = app.get_webview_window("main") { configure_windows_context_menu(&main_window)?; }

            let show_item = MenuItem::with_id(app, "show", "Open", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show_item, &settings_item, &quit_item])?;
            tray::TrayIconBuilder::new()
                .tooltip("Clipx")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, evt| match evt.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "settings" => {
                        println!("Settings from the tray!");
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {
                        println!("Unknown tray event!");
                    }
                })
                .on_tray_icon_event(|tray, evt| {
                    if let tray::TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = evt
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_this_device_identity,
            get_paired_devices,
            get_available_devices,
            connect_device,
            disconnect_device,
            pair_device,
            set_auto_connect,
            forget_device,
            get_clipboard_history,
            remove_clipboard_entry,
            clear_clipboard_history,
            send_text,
            send_rich_text,
            send_image,
            send_file,
            pick_files,
            get_dropped_files,
            send_file_path,
            download_clipboard_file,
            reveal_file_location,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<AppState>();
                tauri::async_runtime::block_on(async {
                    let mut lock = state.ipc.lock().await;
                    let _ = lock.shutdown().await;
                });
            }
        });
}

#[tauri::command]
fn reveal_file_location(path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(path);
    if !path.is_file() { return Err("The saved file is no longer available".into()); }
    #[cfg(windows)] { std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn().map_err(|e| format!("Could not open File Explorer: {e}"))?; return Ok(()); }
    #[cfg(target_os = "macos")] { std::process::Command::new("open").arg("-R").arg(&path).spawn().map_err(|e| format!("Could not reveal file: {e}"))?; return Ok(()); }
    #[cfg(all(unix, not(target_os = "macos")))] { std::process::Command::new("xdg-open").arg(path.parent().unwrap_or_else(|| std::path::Path::new("."))).spawn().map_err(|e| format!("Could not open file location: {e}"))?; return Ok(()); }
    #[allow(unreachable_code)] Err("Opening file locations is not supported on this platform".into())
}

#[tauri::command]
async fn get_this_device_identity(
    state: State<'_, AppState>,
) -> Result<types::OwnIdentity, String> {
    state.ipc.lock().await.get_this_device_identity().await
}

#[tauri::command]
async fn get_paired_devices(
    state: State<'_, AppState>,
) -> Result<Vec<types::PairedDevice>, String> {
    state.ipc.lock().await.get_paired_devices().await
}

#[tauri::command]
async fn get_available_devices(
    state: State<'_, AppState>,
) -> Result<Vec<types::AvailableDevice>, String> {
    state.ipc.lock().await.get_available_devices().await
}

#[tauri::command]
async fn connect_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.connect_device(device_id).await
}

#[tauri::command]
async fn disconnect_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<String, String> {
    state.ipc.lock().await.disconnect_device(device_id).await
}

#[tauri::command]
async fn pair_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.pair_device(device_id).await
}

#[tauri::command]
async fn set_auto_connect(
    state: State<'_, AppState>,
    device_id: String,
    auto_connect: bool,
) -> Result<bool, String> {
    state
        .ipc
        .lock()
        .await
        .set_auto_connect(device_id, auto_connect)
        .await
}

#[tauri::command]
async fn forget_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.forget_device(device_id).await
}

#[tauri::command]
async fn get_clipboard_history(state: State<'_, AppState>, limit: Option<u32>) -> Result<Vec<types::ClipHistoryEntry>, String> {
    state.ipc.lock().await.get_clipboard_history(limit).await
}

#[tauri::command]
async fn remove_clipboard_entry(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.ipc.lock().await.remove_clipboard_entry(id).await
}

#[tauri::command]
async fn clear_clipboard_history(state: State<'_, AppState>) -> Result<(), String> {
    state.ipc.lock().await.clear_clipboard_history().await
}
#[tauri::command]
async fn send_text(state: State<'_, AppState>, content: String) -> Result<String, String> {
    state.ipc.lock().await.send_text(content).await
}

#[tauri::command]
async fn send_image(state: State<'_, AppState>, width: u32, height: u32, rgba: Vec<u8>) -> Result<String, String> {
    state.ipc.lock().await.send_image(width, height, rgba).await
}

#[tauri::command]
async fn send_rich_text(state: State<'_, AppState>, text: String, html: String) -> Result<String, String> {
    state.ipc.lock().await.send_rich_text(text, html).await
}

#[tauri::command]
async fn send_file(state: State<'_, AppState>, name: String, mime_type: String, data: Vec<u8>) -> Result<String, String> {
    state.ipc.lock().await.send_file(name, mime_type, data).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedFile {
    path: String,
    name: String,
    size: u64,
    mime_type: String,
}

#[tauri::command]
fn pick_files() -> Vec<PickedFile> {
    rfd::FileDialog::new().pick_files().unwrap_or_default().into_iter().filter_map(|path| {
        let metadata = std::fs::metadata(&path).ok()?;
        if !metadata.is_file() { return None; }
        let name = path.file_name()?.to_string_lossy().into_owned();
        Some(PickedFile { path: path.to_string_lossy().into_owned(), name: name.clone(), size: metadata.len(), mime_type: mime_from_name(&name) })
    }).collect()
}

#[tauri::command]
fn get_dropped_files(paths: Vec<String>) -> Vec<PickedFile> {
    paths.into_iter().filter_map(|raw| {
        let path = std::path::PathBuf::from(raw);
        let metadata = std::fs::metadata(&path).ok()?;
        if !metadata.is_file() { return None; }
        let name = path.file_name()?.to_string_lossy().into_owned();
        Some(PickedFile { path: path.to_string_lossy().into_owned(), name: name.clone(), size: metadata.len(), mime_type: mime_from_name(&name) })
    }).collect()
}

#[tauri::command]
async fn send_file_path(state: State<'_, AppState>, name: String, mime_type: String, path: String) -> Result<String, String> {
    state.ipc.lock().await.send_file_path(name, mime_type, path).await
}

#[tauri::command]
async fn download_clipboard_file(state: State<'_, AppState>, id: String) -> Result<String, String> {
    state.ipc.lock().await.download_clipboard_file(id).await
}
