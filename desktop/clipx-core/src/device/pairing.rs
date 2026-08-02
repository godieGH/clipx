// device/pairing.rs
use super::trusted::TrustedDeviceStore;
use super::types::{SeenDevice, TrustedDevice};
use std::time::SystemTime;

pub enum PairingDecision {
    AutoApproved,
    AwaitingUserApproval,
}

/// Decides what should happen when we see an untrusted device.
/// For the prototype: always requires manual approval — no auto-pairing logic yet.
pub fn evaluate(seen: &SeenDevice, trusted: &TrustedDeviceStore) -> PairingDecision {
    if trusted.is_trusted(&seen.id) {
        // shouldn't normally be called in this case, but defensive
        return PairingDecision::AutoApproved;
    }
    PairingDecision::AwaitingUserApproval
}

/// Called once a pairing decision is made (e.g. via CLI `--pair <id>`).
pub fn approve(seen: &SeenDevice, trusted: &mut TrustedDeviceStore) {
    trusted.trust(TrustedDevice {
        id: seen.id.clone(),
        name: seen.name.clone(),
        device_type: seen.device_type,
        paired_at: SystemTime::now(),
        public_key: seen.public_key
    });
}