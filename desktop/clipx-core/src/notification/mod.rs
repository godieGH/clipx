//! Notification engine: the one place in core that knows how to ask the
//! user a question via a native OS prompt.
//
//! Callers use domain-specific decisions such as `PairDecision` or
//! `IncomingClipboardDecision`. Platform-specific details stay inside
//! the platform notification engine.

#[cfg(target_os = "windows")]
mod windows_engine;
#[cfg(target_os = "windows")]
pub use windows_engine::NotificationEngine;

#[cfg(not(target_os = "windows"))]
mod stub_engine;
#[cfg(not(target_os = "windows"))]
pub use stub_engine::NotificationEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairDecision {
    Allow,
    Deny,

    /// Timed out, dismissed without a button click, or the prompt
    /// could not be shown.
    NoResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingClipboardDecision {
    Copy,
    Ignore,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub enum Prompt {
    PairRequest {
        peer_name: String,
    },

    ConfirmCode {
        peer_name: String,
        code: String,
    },

    IncomingClipboard {
        peer_name: String,
        content: String, // later will be use to for image url,
        // clipboard_type: ClipboardType, // Text, Image, File, RichText
    },
}

impl Prompt {
    pub fn render(&self) -> (String, String) {
        match self {
            Prompt::PairRequest { peer_name } => (
                "Pairing request".to_string(),
                format!("{peer_name} wants to pair with this device."),
            ),

            Prompt::ConfirmCode { peer_name, code } => (
                "Confirm pairing code".to_string(),
                format!(
                    "Code from {peer_name}: {code}\nDoes this match on both devices?"
                ),
            ),

            Prompt::IncomingClipboard { peer_name, .. } => (
                "Clipboard received".to_string(),
                format!(
                    "Received clipboard from {peer_name}. \
                     Would you like to copy it to your clipboard?"
                ),
            ),
        }
    }
}