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
        let devices = match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(devices) => devices,
                Err(error) => {
                    tracing::warn!(%error, "trusted-device store is invalid; starting empty");
                    HashMap::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => {
                tracing::warn!(%error, "could not read trusted-device store; starting empty");
                HashMap::new()
            }
        };
        tracing::debug!(devices = devices.len(), "trusted-device store loaded");

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

    pub fn list(&self) -> impl Iterator<Item = &TrustedDevice> {
        self.devices.values()
    }

    /// Adds a device to the trusted list and persists immediately.
    pub fn trust(&mut self, device: TrustedDevice) {
        let device_id = device.id.clone();
        self.devices.insert(device_id.clone(), device);
        tracing::info!(%device_id, "device added to trusted store");
        if let Err(e) = self.save() {
            tracing::error!("failed to persist trusted devices: {e}");
        }
    }

    /// Removes a device's trust and persists immediately.
    pub fn revoke(&mut self, device_id: &str) {
        let removed = self.devices.remove(device_id).is_some();
        if removed {
            tracing::info!(%device_id, "device removed from trusted store");
        }
        if let Err(e) = self.save() {
            tracing::error!("failed to persist trusted devices: {e}");
        }
    }

    pub fn set_auto_connect(&mut self, device_id: &str, auto_connect: bool) -> bool {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.auto_connect = auto_connect;
            tracing::debug!(%device_id, auto_connect, "trusted-device auto-connect setting changed");
            if let Err(e) = self.save() {
                tracing::error!("failed to persist trusted devices: {e}");
            }
            true
        } else {
            false
        }
    }
}
