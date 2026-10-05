use crate::message::proto::DeviceType;
use directories::ProjectDirs;
use std::sync::OnceLock;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MACHINE_DEVICE_ID_FILE: &str = "device_id";
const TRUSTED_DEVICES_FILE: &str = "trusted_devices.json";
const IDENTITY_KEY_FILE: &str = "identity_key";
const WEBSOCKET_SERVER_PORT: u32 = 8080;
const CLIPBOARD_HISTORY_FILE: &str = "clipboard_history.json";
static CONFIG_ROOT: OnceLock<PathBuf> = OnceLock::new();
static DEVICE_NAME_OVERRIDE: OnceLock<String> = OnceLock::new();
const MIGRATED_MARKER: &str = ".migrated";
const STATE_FILES: [&str; 4] = [
    MACHINE_DEVICE_ID_FILE,
    TRUSTED_DEVICES_FILE,
    IDENTITY_KEY_FILE,
    CLIPBOARD_HISTORY_FILE,
];
static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
static TRANSFER_DIR: OnceLock<PathBuf> = OnceLock::new();

/// For mobile Oses — Android/Ios have a real human-readable device name
/// that only the platform layer can resolve, so it's supplied here the
/// same way the config root is
pub fn set_device_name_override(name: String) {
    let _ = DEVICE_NAME_OVERRIDE.set(name);
}

pub fn get_hostname() -> String {
    if let Some(name) = DEVICE_NAME_OVERRIDE.get() {
        return name.clone();
    }
    hostname::get().unwrap().to_string_lossy().into_owned()
}

fn project_dirs() -> ProjectDirs {
    ProjectDirs::from("com", "godiegh", "clipx").unwrap()
}

/// Identity, trusted devices, history. Per-machine, never roaming.
fn state_dir() -> PathBuf {
    let dir = STATE_DIR.get_or_init(|| {
        if let Some(root) = CONFIG_ROOT.get() {
            return root.clone(); // mobile: unchanged
        }
        let dirs = project_dirs();
        let legacy = dirs.config_dir().to_path_buf();
        if !cfg!(windows) {
            return legacy; // already per-machine here
        }
        let target = dirs.data_local_dir().to_path_buf();
        match migrate_state(&legacy, &target) {
            Ok(()) => target,
            Err(e) => {
                tracing::error!(error = %e, "state migration failed; staying on legacy dir");
                legacy
            }
        }
    });
    fs::create_dir_all(dir).unwrap();
    dir.clone()
}

/// Parent of `clipboard-files` and `incoming-files`. Temporary data.
pub fn transfer_dir() -> PathBuf {
    TRANSFER_DIR
        .get_or_init(|| match CONFIG_ROOT.get() {
            Some(root) => root.clone(), // mobile: unchanged
            None => {
                let dirs = project_dirs();
                purge_legacy_transfer_dirs(dirs.config_dir());
                dirs.cache_dir().to_path_buf()
            }
        })
        .clone()
}

fn purge_legacy_transfer_dirs(legacy: &Path) {
    for d in ["clipboard-files", "incoming-files"] {
        let _ = fs::remove_dir_all(legacy.join(d));
    }
}

fn migrate_state(legacy: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    let marker = target.join(MIGRATED_MARKER);
    if marker.exists() || !legacy.is_dir() {
        return Ok(());
    }

    let mut created = Vec::new(); // new copies, for rollback
    let mut copied = Vec::new(); // legacy originals, deleted after commit
    let result = (|| -> std::io::Result<()> {
        for name in STATE_FILES {
            let (from, to) = (legacy.join(name), target.join(name));
            if !from.is_file() || to.exists() {
                continue; // target wins, never overwrite
            }
            let tmp = target.join(format!("{name}.migrating"));
            fs::copy(&from, &tmp)?;
            fs::rename(&tmp, &to)?;
            created.push(to);
            copied.push(from);
        }
        fs::write(&marker, b"") // commit point
    })();

    if let Err(e) = result {
        for p in &created {
            let _ = fs::remove_file(p); // roll back
        }
        return Err(e);
    }

    for p in copied {
        let _ = fs::remove_file(p);
    }
    purge_legacy_transfer_dirs(legacy);
    let _ = fs::remove_dir(legacy); // only succeeds if empty
    tracing::info!(from = ?legacy, to = ?target, "migrated state to local data dir");
    Ok(())
}

/// This is for mobile OSes — since they need a way to tell the core
/// where the config dir resides, this is different from desktops which already handles it
pub fn set_config_root(path: PathBuf) {
    let _ = CONFIG_ROOT.set(path);
}

#[allow(unused)]
pub fn machine_device_id_path() -> PathBuf {
    state_dir().join(MACHINE_DEVICE_ID_FILE)
}

pub fn trusted_devices_path() -> PathBuf {
    state_dir().join(TRUSTED_DEVICES_FILE)
}

pub fn identity_key_path() -> PathBuf {
    state_dir().join(IDENTITY_KEY_FILE)
}

pub fn clipboard_history_path() -> PathBuf {
    state_dir().join(CLIPBOARD_HISTORY_FILE)
}

pub fn get_ws_port() -> u32 {
    WEBSOCKET_SERVER_PORT
}

pub fn get_current_device_type() -> DeviceType {
    #[cfg(target_os = "android")]
    return DeviceType::Android;

    #[cfg(target_os = "windows")]
    return DeviceType::Windows;

    #[cfg(target_os = "linux")]
    return DeviceType::Linux;

    #[cfg(target_os = "macos")]
    return DeviceType::Macos;

    #[cfg(target_os = "ios")]
    return DeviceType::Ios;

    #[cfg(not(any(
        target_os = "android",
        target_os = "windows",
        target_os = "linux",
        target_os = "macos",
        target_os = "ios"
    )))]
    return DeviceType::Unspecified;
}

pub fn local_ipv4_candidates() -> Vec<(String, std::net::Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.addr.ip() {
            std::net::IpAddr::V4(v4) if v4.is_private() => Some((iface.name, v4)),
            _ => None,
        })
        .collect()
}

pub fn preferred_local_ip() -> Option<std::net::Ipv4Addr> {
    let mut candidates = local_ipv4_candidates();
    // Rank: prefer wired-sounding names over Wi-Fi/virtual-sounding ones.
    // Cheap heuristic, not perfect — good enough until this is observed to pick wrong.
    candidates.sort_by_key(|(name, _)| {
        let n = name.to_lowercase();
        if n.contains("eth") || n.contains("ethernet") {
            0
        } else if n.contains("wl") || n.contains("wifi") || n.contains("wi-fi") {
            1
        } else if n.contains("vmnet")
            || n.contains("vboxnet")
            || n.contains("docker")
            || n.contains("tun")
            || n.contains("veth")
        {
            3
        } else {
            2
        }
    });
    candidates.into_iter().next().map(|(_, ip)| ip)
}
