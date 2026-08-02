use std::{fs, path::PathBuf};
use directories::ProjectDirs;
use uuid::Uuid;

const MACHINE_DEVICE_ID_FILE: &str = "device_id";
const TRUSTED_DEVICES_FILE: &str = "trusted_devices.json";

pub fn get_or_create_device_id(device_id_path: PathBuf) -> String {
    if let Ok(existing) = fs::read_to_string(&device_id_path) {
        return existing.trim().to_string();
    }
    let new_id = Uuid::new_v4().to_string();
    fs::write(device_id_path, &new_id).expect("failed to persist device id");
    new_id
}

pub fn get_hostname() -> String {
    hostname::get()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn config_dir() -> PathBuf {
    let dirs = ProjectDirs::from(
        "com",
        "godiegh",
        "clipx").unwrap();
    fs::create_dir_all(dirs.config_dir()).unwrap();
    dirs.config_dir().to_path_buf()
}

pub fn machine_device_id_path() -> PathBuf {
    config_dir().join(MACHINE_DEVICE_ID_FILE)
}

pub fn trusted_devices_path() -> PathBuf {
    config_dir().join(TRUSTED_DEVICES_FILE)
}