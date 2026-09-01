//! When platforms wants to talk to core — core exposes some useful traits
//! these traits provide a way of platform like android, ios, linux etc.
//! to provide a way that core can access their clipboard, notification plus
//! registering events

use crate::notification::{IncomingClipboardDecision, PairDecision};

/// platform implements this to give an object that writes to the system clipboard
pub trait ClipboardSink: Send + Sync {
    fn write(&self, content: String);
}

/// platform implements this to give core access to platform notification system
pub trait NotificationPrompter: Send + Sync {
    fn ask_pair_request(&self, peer_name: String) -> PairDecision;
    fn ask_pair_code(&self, peer_name: String, code: String) -> PairDecision;
    fn ask_received_clipboard(&self, peer_name: String) -> IncomingClipboardDecision;
    fn notify_info(&self, title: String, message: String);
}

/// platform implements this so core knows how to push events to them
/// when something happens
pub trait CoreEventListener: Send + Sync {
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
}

pub type PushEvents = crate::message::proto::clipx::IpcEvent;