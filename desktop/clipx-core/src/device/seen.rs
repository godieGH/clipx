use super::types::SeenDevice;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SeenDeviceRegistry {
    devices: HashMap<String, SeenDevice>,
}

impl SeenDeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or refresh a seen device's last_seen/addr.
    pub fn upsert(&mut self, device: SeenDevice) {
        self.devices.insert(device.id.clone(), device);
    }

    pub fn get(&self, device_id: &str) -> Option<&SeenDevice> {
        self.devices.get(device_id)
    }

    pub fn list(&self) -> impl Iterator<Item = &SeenDevice> {
        self.devices.values()
    }

    /// Remove devices we haven't heard from in `max_age` — they've likely
    /// gone offline or left the network without a graceful goodbye.
    pub fn prune_stale(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.devices
            .retain(|_, d| now.duration_since(d.last_seen) < max_age);
    }
}
