//! Notification engine: the one place in core that knows how to ask the
//! user a question via a native OS prompt.
//
//! Callers use domain-specific decisions such as `PairDecision` or
//! `IncomingClipboardDecision`. Platform-specific details stay inside
//! the platform notification engine.

pub mod platform;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingClipboardKind {
    Text,
    RichText,
    Image,
    File,
}

#[derive(Debug, Clone, Default)]
pub enum ProgressText {
    /// Render the authoritative progress value as a percentage, e.g. `47%`.
    #[default]
    Percentage,
    /// Render the authoritative byte counters, e.g. `47 MB / 100 MB`.
    Bytes,
    /// Render caller-provided presentation text without changing the progress value.
    Custom(String),
    /// Do not render a value string next to the progress bar.
    Hidden,
}

#[derive(Debug, Clone)]
pub struct NotificationConfig {
    pub transfer_title: String,
    pub progress_text: ProgressText,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            transfer_title: "File Transfer".to_string(),
            progress_text: ProgressText::Percentage,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransferNotificationOverride {
    pub transfer_title: Option<String>,
    pub progress_text: Option<ProgressText>,
}

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
        content: String,
        kind: IncomingClipboardKind,
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
                format!("Code from {peer_name}: {code}\nDoes this match on both devices?"),
            ),

            Prompt::IncomingClipboard {
                peer_name,
                content,
                kind,
            } => {
                let body = match kind {
                    IncomingClipboardKind::Text | IncomingClipboardKind::RichText => {
                        format!("{content}\n\nReceived from {peer_name}.")
                    }
                    IncomingClipboardKind::Image => format!("Image received from {peer_name}."),
                    IncomingClipboardKind::File => format!("{content}\nReceived from {peer_name}."),
                };
                ("Clipboard received".to_string(), body)
            }
        }
    }
}
