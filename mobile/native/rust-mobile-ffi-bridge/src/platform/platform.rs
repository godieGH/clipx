#[uniffi::export(with_foreign)]
pub trait ClipboardPlatform: Send + Sync {
    fn write_clipboard(&self, content: String);
    fn write_rich_text(&self, text: String, html: String);
    fn write_image(&self, width: u32, height: u32, rgba: Vec<u8>);
    fn save_file(&self, name: String, mime_type: String, data: Vec<u8>) -> String;
    fn save_file_from_path(&self, name: String, mime_type: String, source_path: String) -> String;
}

#[derive(uniffi::Enum)]
pub enum NotifierDecision {
    PairDecision(u8),
    IncomingClipboardDecision(u8),
}

#[uniffi::export(with_foreign)]
pub trait NotificationPlatform: Send + Sync {
    fn show_pair_request(&self, prompt_id: String, device_name: String);
    fn show_pair_code(&self, prompt_id: String, device_name: String, code: String);
    fn show_received_clipboard(&self, prompt_id: String, device_name: String, action: String);
    fn notify_info(&self, title: String, message: String);
}

#[uniffi::export(with_foreign)]
pub trait ClipxEventListener: Send + Sync {
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
    fn on_pairing_change(&self, device_id: String, state: u8, message: String);
    fn on_file_transfer(&self, entry_id: String, file_id: String, done: u64, total: u64, state: String, message: String);
}
