mod error_dialog;

use clipx_tray_lib::{ipc, message::types, message::IpcCmddBridge};
use tauri::{async_runtime::Mutex, Manager, State};

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

    let state = AppState { ipc: Mutex::new(client) };

    tauri::Builder::default()
        .manage(state)
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
async fn get_this_device_identity(state: State<'_, AppState>) -> Result<types::OwnIdentity, String> {
    state.ipc.lock().await.get_this_device_identity().await
}

#[tauri::command]
async fn get_paired_devices(state: State<'_, AppState>) -> Result<Vec<types::PairedDevice>, String> {
    state.ipc.lock().await.get_paired_devices().await
}

#[tauri::command]
async fn get_available_devices(state: State<'_, AppState>) -> Result<Vec<types::AvailableDevice>, String> {
    state.ipc.lock().await.get_available_devices().await
}

#[tauri::command]
async fn connect_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.connect_device(device_id).await
}

#[tauri::command]
async fn disconnect_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.disconnect_device(device_id).await
}

#[tauri::command]
async fn pair_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.pair_device(device_id).await
}

#[tauri::command]
async fn set_auto_connect(state: State<'_, AppState>, device_id: String, auto_connect: bool) -> Result<bool, String> {
    state.ipc.lock().await.set_auto_connect(device_id, auto_connect).await
}

#[tauri::command]
async fn forget_device(state: State<'_, AppState>, device_id: String) -> Result<String, String> {
    state.ipc.lock().await.forget_device(device_id).await
}