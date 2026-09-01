//! this stands as middle man between the core and the platforms uniffi
//! so core doesn't know anything about uniffi object 

use std::sync::Arc;
use super::{ClipboardPlatform, NotificationPlatform, ClipxEventListener, NotifierDecision};

#[allow(unused)]
pub struct ClipboardSinkAdapter(Arc<dyn ClipboardPlatform>);
impl clipx_core::platform::ClipboardSink for ClipboardSinkAdapter {
    fn write(&self, content: String) {
        self.0.write_clipboard(content);
    }
}

#[allow(unused)]
pub struct NotificationAdapter(Arc<dyn NotificationPlatform>);
impl clipx_core::platform::NotificationPrompter for NotificationAdapter {
    fn ask_pair_request(&self, peer_name: String) -> clipx_core::notification::PairDecision {
        match self.0.show_pairing_request_prompt(peer_name) {
            NotifierDecision::PairDecision(d) => {
                match d {
                    0 => clipx_core::notification::PairDecision::Allow,
                    1 => clipx_core::notification::PairDecision::Deny,
                    _ => clipx_core::notification::PairDecision::NoResponse
                }
            }
            _ => {
                clipx_core::notification::PairDecision::NoResponse
            }
        }
    }
    fn ask_pair_code(&self, peer_name: String, code: String) -> clipx_core::notification::PairDecision {
        // pair code also use same as pair request just different message 
        match self.0.show_pairing_code_prompt(peer_name, code) {
            NotifierDecision::PairDecision(d) => {
                match d {
                    0 => clipx_core::notification::PairDecision::Allow,
                    1 => clipx_core::notification::PairDecision::Deny,
                    _ => clipx_core::notification::PairDecision::NoResponse
                }
            }
            _ => {
                clipx_core::notification::PairDecision::NoResponse
            }
        }
    }

    fn ask_received_clipboard(&self, peer_name: String) -> clipx_core::notification::IncomingClipboardDecision {
        match self.0.show_received_clipboard_prompt(peer_name) {
            NotifierDecision::IncomingClipboardDecision(d) => {
                match d {
                    0 => clipx_core::notification::IncomingClipboardDecision::Copy,
                    _ => clipx_core::notification::IncomingClipboardDecision::Ignore,
                }
            }
            _ => {
                clipx_core::notification::IncomingClipboardDecision::Ignore
            }
        }
    }

    fn notify_info(&self, title: String, message: String) {
        self.0.notify_info(title, message);
    }
}

#[allow(unused)]
pub struct ClipxEventAdapter(Arc<dyn ClipxEventListener>);
impl clipx_core::platform::CoreEventListener for ClipxEventAdapter {
    fn on_clipboard_change(&self) {
        self.0.on_clipboard_change();
    }

    fn on_device_change(&self) {
        self.0.on_device_change();
    }
}