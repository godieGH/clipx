use directories::ProjectDirs;
use interprocess::local_socket::Name;
use std::{io, path::PathBuf};

fn dirs() -> io::Result<ProjectDirs> {
    ProjectDirs::from("com", "godiegh", "clipx")
        .ok_or_else(|| io::Error::other("no home directory"))
}

#[cfg(unix)]
fn runtime_dir() -> io::Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(d) => Ok(PathBuf::from(d)),
        None => Ok(dirs()?.config_dir().to_path_buf()),
    }
}

#[cfg(unix)]
pub fn socket_path(name: &str) -> io::Result<PathBuf> {
    Ok(runtime_dir()?.join(format!("{name}.sock")))
}

#[cfg(windows)]
pub fn endpoint_name(name: &str) -> io::Result<Name<'static>> {
    use interprocess::local_socket::{GenericNamespaced, ToNsName};
    name.to_string().to_ns_name::<GenericNamespaced>()
}

#[cfg(unix)]
pub fn endpoint_name(name: &str) -> io::Result<Name<'static>> {
    use interprocess::local_socket::{GenericFilePath, ToFsName};
    socket_path(name)?.to_fs_name::<GenericFilePath>()
}

/// One token file per endpoint name, so several endpoints (tests) never clash.
pub fn token_path(name: &str) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(dirs()?.data_local_dir().join(format!("{name}.token")))
    }
    #[cfg(unix)]
    {
        Ok(runtime_dir()?.join(format!("{name}.token")))
    }
}

/// `Authorization` header value the core expects on the WebSocket upgrade.
pub fn bearer(name: &str) -> io::Result<String> {
    Ok(format!(
        "Bearer {}",
        std::fs::read_to_string(token_path(name)?)?.trim()
    ))
}
