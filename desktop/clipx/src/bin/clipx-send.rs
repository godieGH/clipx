#![cfg_attr(windows, windows_subsystem = "windows")]

use clipx_lib::{
    ipc::non_blocking::Client,
    message::IpcCmdBridge,
};

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

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

async fn connect_to_core(
    client: &mut Client,
) -> Result<Option<Child>, String> {
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

#[tokio::main]
async fn main() {
    let mut args = std::env::args_os();
    let _program = args.next();

    let Some(raw_path) = args.next() else {
        return;
    };

    if args.next().is_some() {
        // Phase 1 intentionally supports one file only.
        return;
    }

    let path = PathBuf::from(raw_path);

    if !path.is_file() {
        eprintln!("Invalid file path");
        return;
    }

    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };

    let path_string = path.to_string_lossy().into_owned();
    let mime_type = mime_from_name(name);

    let mut client = Client::new("clipx");

    let _owned_core = match connect_to_core(&mut client).await {
        Ok(child) => child,
        Err(_) => return,
    };

    let _ = client
        .send_file_path(
            name.to_string(),
            mime_type,
            path_string,
        )
        .await;

    let _ = client.shutdown().await;
}