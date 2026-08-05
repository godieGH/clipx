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
