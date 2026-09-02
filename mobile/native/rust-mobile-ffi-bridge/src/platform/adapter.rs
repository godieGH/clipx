//! Adapters between UniFFI platform objects and core platform traits.

use std::sync::Arc;

use super::{ClipboardPlatform, ClipxEventListener, NotificationPlatform};

#[derive(Clone)]
pub struct ClipboardSinkAdapter(pub Arc<dyn ClipboardPlatform>);

impl clipx_core::platform::ClipboardSink for ClipboardSinkAdapter {
    fn write_text(&self, content: String) { self.0.write_clipboard(content); }
    fn write_rich_text(&self, text: String, html: String) { self.0.write_rich_text(text, html); }
    fn write_image(&self, width: u32, height: u32, rgba: Vec<u8>) { self.0.write_image(width, height, rgba); }
    fn save_file(&self, name: &str, mime_type: &str, data: &[u8]) -> Result<String, String> { Ok(self.0.save_file(name.to_string(), mime_type.to_string(), data.to_vec())) }
    fn save_file_from_path(&self, name: &str, mime_type: &str, source: &std::path::Path) -> Result<String, String> { Ok(self.0.save_file_from_path(name.to_string(), mime_type.to_string(), source.to_string_lossy().into_owned())) }
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

    fn show_received_clipboard(&self, prompt_id: String, peer_name: String, action: String) {
        self.0.show_received_clipboard(prompt_id, peer_name, action);
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

    fn on_pairing_change(&self, device_id: String, state: u8, message: String) { self.0.on_pairing_change(device_id, state, message); }
    fn on_file_transfer(&self, entry_id: String, file_id: String, done: u64, total: u64, state: String, message: String) { self.0.on_file_transfer(entry_id, file_id, done, total, state, message); }
}
