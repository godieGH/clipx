use clipx_tray_lib::{ipc, message::types, message::IpcCmddBridge};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_this_device_identity])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}


#[tauri::command]
fn get_this_device_identity() -> Result<types::OwnIdentity, String> {
    let mut ipc = ipc::IpcClient::new("clipx").run();
    // the command bridge knows what to do under the hood
    Ok(ipc.get_this_device_identity()?)
}