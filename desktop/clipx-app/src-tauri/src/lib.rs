use clipx_tray_lib::{ipc, message::types, message::IpcCmddBridge};
use tauri::{async_runtime::Mutex, Manager, State};

pub struct AppState {
    ipc: Mutex<ipc::non_blocking::Client>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::async_runtime::block_on(async {
        let mut client = ipc::non_blocking::Client::new("clipx");
        client
            .start()
            .await
            .unwrap_or_else(|e| panic!("Failed to start IPC: {e}"));

        let state = AppState {
            ipc: Mutex::new(client),
        };

        tauri::Builder::default()
            .manage(state)
            .plugin(tauri_plugin_opener::init())
            .invoke_handler(tauri::generate_handler![get_this_device_identity])
            .build(tauri::generate_context!())
            .expect("error while building tauri application")
            .run(|app_handle, event| {
                if let tauri::RunEvent::Exit = event {
                    let state = app_handle.state::<AppState>();
                    tauri::async_runtime::block_on(async {
                        let mut lock = state.ipc.lock().await;
                        let _ = lock.shutdown().await; // best-effort, don't panic on the way out
                    });
                }
            });
    });
}

#[tauri::command]
async fn get_this_device_identity(
    state: State<'_, AppState>,
) -> Result<types::OwnIdentity, String> {
    let mut ipc = state.ipc.lock().await;
    Ok(ipc.get_this_device_identity().await?)
}
