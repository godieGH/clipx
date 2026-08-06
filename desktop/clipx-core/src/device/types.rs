use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::{Instant, SystemTime};

// just reuse from the protobuf def.
pub use crate::message::proto::DeviceType;

/// A device that has been paired and is persisted to disk.
/// Identity only — no connection state, no address (that's session-specific).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub id: String,
    pub name: String,
    pub device_type: DeviceType,
    pub paired_at: SystemTime,
    pub public_key: [u8; 32],
}

/// A device currently visible via discovery. Purely in-memory, ephemeral —
/// rebuilt from scratch every time core starts, gone once it stops broadcasting.
#[derive(Debug, Clone)]
pub struct SeenDevice {
    pub id: String,
    pub name: String,
    pub device_type: DeviceType,
    pub addr: SocketAddr,
    pub last_seen: Instant,
}

#[derive(Debug, Clone)]
pub struct IdentitySnapshot {
    pub device_id: String,
    pub device_name: String,
    pub public_key_hex: String,
    pub ws_port: u32,
    pub device_type: DeviceType,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_snapshot_uses_provided_name_and_port() {
        let snapshot = IdentitySnapshot {
            device_id: "abc123".to_string(),
            device_name: "my-host".to_string(),
            public_key_hex: "deadbeef".to_string(),
            ws_port: 9000,
            device_type: DeviceType::Windows,
        };

        assert_eq!(snapshot.device_name, "my-host");
        assert_eq!(snapshot.ws_port, 9000);
        assert_eq!(snapshot.device_type, DeviceType::Windows);
    }
}
