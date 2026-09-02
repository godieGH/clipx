//! Adapters between UniFFI platform objects and core platform traits.

use std::sync::Arc;

use super::{ClipboardPlatform, ClipxEventListener, NotificationPlatform};

#[derive(Clone)]
pub struct ClipboardSinkAdapter(pub Arc<dyn ClipboardPlatform>);

impl clipx_core::platform::ClipboardSink for ClipboardSinkAdapter {
    fn write(&self, content: String) {
        self.0.write_clipboard(content);
    }
}

#[derive(Clone)]
pub struct NotificationAdapter(pub Arc<dyn NotificationPlatform>);

impl clipx_core::platform::NotificationPrompter for NotificationAdapter {
    fn show_pair_request(&self, prompt_id: String, peer_name: String) {
        self.0.show_pair_request(prompt_id, peer_name);
    }

    fn show_pair_code(&self, prompt_id: String, peer_name: String, code: String) {
        self.0.show_pair_code(prompt_id, peer_name, code);
    }

    fn show_received_clipboard(&self, prompt_id: String, peer_name: String) {
        self.0.show_received_clipboard(prompt_id, peer_name);
    }

    fn notify_info(&self, title: String, message: String) {
        self.0.notify_info(title, message);
    }
}

#[derive(Clone)]
pub struct ClipxEventAdapter(pub Arc<dyn ClipxEventListener>);

impl clipx_core::platform::CoreEventListener for ClipxEventAdapter {
    fn on_clipboard_change(&self) {
        self.0.on_clipboard_change();
    }

    fn on_device_change(&self) {
        self.0.on_device_change();
    }
}
