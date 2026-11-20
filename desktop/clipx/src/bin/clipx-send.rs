#![cfg_attr(windows, windows_subsystem = "windows")]

use clipx_lib::{ipc::non_blocking::Client, message::IpcCmdBridge};

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

const DEVICE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(300);

fn mime_from_name(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn core_path() -> Result<PathBuf, String> {
    let dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("unable to locate executable directory")?
        .to_path_buf();

    Ok(dir.join(if cfg!(windows) {
        "clipx-core.exe"
    } else {
        "clipx-core"
    }))
}

fn spawn_core() -> Result<Child, String> {
    let mut cmd = Command::new(core_path()?);

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    cmd.spawn()
        .map_err(|e| format!("could not start clipx-core: {e}"))
}

async fn connect_to_core(client: &mut Client) -> Result<Option<Child>, String> {
    if client.start().await.is_ok() {
        return Ok(None);
    }

    let mut owned_core = spawn_core()?;
    let mut last_error = String::from("unable to connect to clipx-core");

    for _ in 0..20 {
        match client.start().await {
            Ok(()) => return Ok(Some(owned_core)),
            Err(error) => {
                last_error = error;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }

    let _ = owned_core.kill();
    let _ = owned_core.wait();

    Err(last_error)
}

/// Wait for discovery/auto-connect to finish before asking the core to offer the file.
/// This deliberately respects each paired device's persisted auto-connect preference.
async fn wait_for_connected_peer(client: &mut Client) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + DEVICE_CONNECTION_TIMEOUT;
    let mut connect_requested = HashSet::new();

    loop {
        let devices = client.get_paired_devices().await?;

        if devices
            .iter()
            .any(|device| device.connection == "connected")
        {
            return Ok(());
        }

        // Only initiate connections for devices whose saved preference permits
        // auto-connect. This also handles the case where the core was already
        // running and had discovered a peer before this helper was launched.
        for device in &devices {
            if device.auto_connect
                && device.connection == "disconnected"
                && !device.ip_address.is_empty()
                && connect_requested.insert(device.fingerprint.clone())
            {
                if let Err(error) = client.connect_device(device.fingerprint.clone()).await {
                    eprintln!(
                        "ClipX: could not initiate connection to {}: {}",
                        device.name, error
                    );
                }
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(connection_timeout_message(&devices));
        }

        tokio::time::sleep(DEVICE_POLL_INTERVAL).await;
    }
}

fn connection_timeout_message(devices: &[clipx_lib::message::types::PairedDevice]) -> String {
    if devices.is_empty() {
        return "No paired ClipX devices were found. Open ClipX and pair a device before sending files.".into();
    }

    if devices
        .iter()
        .all(|device| device.connection == "unavailable")
    {
        return "No paired ClipX devices are currently reachable. Make sure ClipX is running on the receiving device and both devices can discover each other.".into();
    }

    "No ClipX device connected within 15 seconds. Open ClipX and connect a paired device, or enable auto-connect for that device, then try again.".into()
}

#[cfg(windows)]
fn show_error(error: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let message = format!("ClipX could not send this file.\n\n{error}");
    let message: Vec<u16> = std::ffi::OsStr::new(&message)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = std::ffi::OsStr::new("ClipX")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error(error: &str) {
    eprintln!("ClipX: {error}");
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args_os();
    let _program = args.next();

    let Some(raw_path) = args.next() else {
        return Err("No file path was provided.".into());
    };

    if args.next().is_some() {
        return Err("This version of Send with ClipX supports one file at a time. Select a single file and try again.".into());
    }

    let path = PathBuf::from(raw_path);

    if !path.is_file() {
        return Err(format!(
            "The selected path is not a file: {}",
            path.display()
        ));
    }

    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err("The selected file has an invalid or unsupported name.".into());
    };

    let path_string = path.to_string_lossy().into_owned();
    let mime_type = mime_from_name(name);
    let mut client = Client::new("clipx");

    // If this helper started the core, keep its child handle alive through the send.
    // Dropping the Child handle does not stop the background process.
    let _owned_core = connect_to_core(&mut client).await?;

    let send_result = async {
        wait_for_connected_peer(&mut client).await?;
        client
            .send_file_path(name.to_string(), mime_type, path_string)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not offer the file: {error}"))
    }
    .await;

    let _ = client.shutdown().await;
    send_result
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        show_error(&error);
    }
}
