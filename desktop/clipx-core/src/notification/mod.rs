//! Notification engine: the one place in core that knows how to ask the
//! user a yes/no question via a native OS prompt. Callers (DeviceManager,
//! and anything else later) only ever see `Prompt`/`PairDecision` — no
//! platform-specific vocabulary (toast XML, button ids, WinRT quirks)
//! crosses this module's boundary in either direction.

#[cfg(target_os = "windows")]
mod windows_engine;
#[cfg(target_os = "windows")]
pub use windows_engine::NotificationEngine;

#[cfg(not(target_os = "windows"))]
mod stub_engine;
#[cfg(not(target_os = "windows"))]
pub use stub_engine::NotificationEngine;

#[allow(unused)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairDecision {
    Allow,
    Deny,
    /// Timed out, dismissed without a button click, or the prompt couldn't
    /// be shown at all (unsupported platform, OS-level failure).
    NoResponse,
}

/// Closed set of things this module knows how to ask the user. Add a
/// variant (and its match arm in each platform engine) when a genuinely new
/// use case shows up — resist a fully generic "caller builds the prompt"
/// escape hatch, since that would leak platform detail back out of here.
#[allow(unused)]
#[derive(Debug, Clone)]
pub enum Prompt {
    PairRequest { peer_name: String },
    ConfirmCode { peer_name: String, code: String },
}