use clipx_tray_lib::{ipc, message::types, message::IpcCmddBridge};
use tauri::{State, async_runtime::Mutex};

type IpcMutex = Mutex<ipc::non_blocking::Client>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Now the ipc is managed as global mutex value and it uses the tauri async mutex to ensure proper locks across await boundaries
        .manage(Mutex::new(ipc::non_blocking::Client::new("clipx")))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_this_device_identity])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}


#[tauri::command]
async fn get_this_device_identity(state: State<'_, IpcMutex>) -> Result<types::OwnIdentity, String> {
    let mut ipc = state.lock().await;

    // this guard ensures the service is running, because by default the state doesn't start the the service due to
    // I was not able to know how to call run and await it the the setup hook safely that would need an async setup cb
    // which is impossible unless wrap it in an async spawned task inside setup cb, the hook only takes a syncronous clouser |app, api?| {}
    // calling run multiple times is a no-op has no effects in both ipc::Client(the blocking) and ipc::non_blocking Clients
    // but the blocking version consumes the instance so you have to call and assign it back from the returned value
    if !ipc.is_running() {
        ipc.run().await?
    }
    
    // the command-bridge knows what to do under the hood
    Ok(ipc.get_this_device_identity().await?)
}