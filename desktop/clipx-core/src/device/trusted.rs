use super::types::TrustedDevice;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct TrustedDeviceStore {
    devices: HashMap<String, TrustedDevice>,
    path: PathBuf,
}

impl TrustedDeviceStore {
    /// Loads from disk if the file exists, otherwise starts empty.
    pub fn load(path: PathBuf) -> Self {
        let devices = fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default();

        Self { devices, path }
    }

    fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.devices)
            .expect("trusted devices should always serialize");
        fs::write(&self.path, json)
    }

    pub fn is_trusted(&self, device_id: &str) -> bool {
        self.devices.contains_key(device_id)
    }

    pub fn get(&self, device_id: &str) -> Option<&TrustedDevice> {
        self.devices.get(device_id)
    }

    #[allow(unused)]
    pub fn list(&self) -> impl Iterator<Item = &TrustedDevice> {
        self.devices.values()
    }

    /// Adds a device to the trusted list and persists immediately.
    pub fn trust(&mut self, device: TrustedDevice) {
        self.devices.insert(device.id.clone(), device);
        if let Err(e) = self.save() {
            tracing::error!("failed to persist trusted devices: {e}");
        }
    }

    #[allow(unused)]
    /// Removes a device's trust and persists immediately.
    pub fn revoke(&mut self, device_id: &str) {
        self.devices.remove(device_id);
        if let Err(e) = self.save() {
            tracing::error!("failed to persist trusted devices: {e}");
        }
    }
    
    pub fn set_auto_connect(&mut self, device_id: &str, auto_connect: bool) -> bool {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.auto_connect = auto_connect;
            if let Err(e) = self.save() {
                tracing::error!("failed to persist trusted devices: {e}");
            }
            true
        } else {
            false
        }
    }
}
