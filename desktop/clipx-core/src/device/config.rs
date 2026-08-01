use std::{fs, path::PathBuf};
use directories::ProjectDirs;
use uuid::Uuid;

const MACHINE_DEVICE_ID_FILE: &str = "device_id";

pub fn machine_device_id_path() -> PathBuf {
    // for testing same device 
    if let Ok(override_path) = std::env::var("CLIPX_CONFIG_DIR") {
        return PathBuf::from(override_path).join("device_id");
    }

    let dirs = ProjectDirs::from(
        "com",
        "godiegh",
        "clipx"
    ).unwrap();
    fs::create_dir_all(dirs.config_dir()).unwrap(); // create if not available
    dirs.config_dir().join(MACHINE_DEVICE_ID_FILE)
}

// other helpers based on device identity
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