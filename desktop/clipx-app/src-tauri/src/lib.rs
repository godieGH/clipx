use clipx_tray_lib::{ipc, message::types, message::IpcCmddBridge};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, get_this_device_identity])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}


#[tauri::command]
fn get_this_device_identity() -> Result<types::OwnIdentity, String> {
    let mut ipc = ipc::IpcClient::new("clipx").run();
    // the command bridge knows what to do under the hood
    Ok(ipc.get_this_device_identity()?)
}