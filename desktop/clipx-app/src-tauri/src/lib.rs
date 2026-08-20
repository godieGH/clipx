mod error_dialog;

use clipx_lib::{clipx, ipc, message::types, message::IpcCmddBridge};
use tauri::{
    async_runtime::Mutex,
    menu::{Menu, MenuItem},
    tray::{self, MouseButton, MouseButtonState},
    Emitter, Manager, State,
};

pub struct AppState {
    ipc: Mutex<ipc::non_blocking::Client>,
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
                        let event_name = match event.event {
                            Some(clipx::ipc_event::Event::DevicesChanged(_)) => "devices-changed",
                            Some(clipx::ipc_event::Event::ClipboardChanged(_)) => "clipboard-changed",
                            None => continue,
                        };
                        let _ = app_handle.emit(event_name, ());
                    }
                });
            }

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
                        println!("Unknow tray event!");
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