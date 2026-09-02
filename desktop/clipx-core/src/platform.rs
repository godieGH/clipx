//! Platform-facing traits implemented by desktop/mobile hosts.

/// Platform implements this to give core an object that writes to the system clipboard.
pub trait ClipboardSink: Send + Sync {
    fn write(&self, content: String);
}

/// Desktop clipboard implementation backed by arboard.
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub struct ArboardClipboardSink;

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl ArboardClipboardSink {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl Default for ArboardClipboardSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl ClipboardSink for ArboardClipboardSink {
    fn write(&self, content: String) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(error) = clipboard.set_text(content) {
                    tracing::warn!(%error, "failed to apply remote clipboard content");
                }
            }
            Err(error) => tracing::warn!(%error, "failed to open clipboard to apply remote content"),
        }
    }
}

/// Platform implements this to let core show an interactive notification.
/// Prompt decisions are delivered later through `notification::platform::resolve_prompt`.
pub trait NotificationPrompter: Send + Sync {
    fn show_pair_request(&self, prompt_id: String, peer_name: String);
    fn show_pair_code(&self, prompt_id: String, peer_name: String, code: String);
    fn show_received_clipboard(&self, prompt_id: String, peer_name: String);
    fn notify_info(&self, title: String, message: String);
}

/// Platform implements this so core can push coarse-grained state changes back to a host UI.
pub trait CoreEventListener: Send + Sync {
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
}

pub type PushEvents = crate::message::proto::clipx::IpcEvent;
