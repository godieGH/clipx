pub mod config;
use std::time;

pub struct Device {
    id: String,
    name: String,
    device_type: DeviceType,
    last_seen: Option<time::Instant>, // this means last time the device communicated with us
    status: Status
}

pub enum DeviceType {
    Windows,
    Android,
    // macos later
}

pub struct DeviceRegistry {
    devices: Vec<Device>
}

// the flow is Offline by default -> Connecting -> [could fail Failed or Connected Online] -> user disconnects -> Disconnecting -> Offline
#[derive(Default)]
pub enum Status {
    #[default]
    Offline,
    Online,
    Connecting,
    Disconnecting,
    Failed(String) // the reason for failure
}

// some implementations to maybe remove device, add, device update status etc. her