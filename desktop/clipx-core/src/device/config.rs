use crate::message::proto::DeviceType;
use directories::ProjectDirs;
use std::{fs, path::PathBuf};

const MACHINE_DEVICE_ID_FILE: &str = "device_id";
const TRUSTED_DEVICES_FILE: &str = "trusted_devices.json";
const IDENTITY_KEY_FILE: &str = "identity_key";
const WEBSOCKET_SERVER_PORT: u32 = 8080;

pub fn get_hostname() -> String {
    hostname::get().unwrap().to_string_lossy().into_owned()
}

fn config_dir() -> PathBuf {
    let dirs = ProjectDirs::from("com", "godiegh", "clipx").unwrap();
    fs::create_dir_all(dirs.config_dir()).unwrap();
    dirs.config_dir().to_path_buf()
}

pub fn machine_device_id_path() -> PathBuf {
    config_dir().join(MACHINE_DEVICE_ID_FILE)
}

pub fn trusted_devices_path() -> PathBuf {
    config_dir().join(TRUSTED_DEVICES_FILE)
}

pub fn identity_key_path() -> PathBuf {
    config_dir().join(IDENTITY_KEY_FILE)
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
