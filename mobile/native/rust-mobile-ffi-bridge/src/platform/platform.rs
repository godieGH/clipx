#[uniffi::export(with_foreign)]
pub trait ClipboardPlatform: Send + Sync {
    fn write_clipboard(&self, content: String);
}

#[derive(uniffi::Enum)]
pub enum NotifierDecision {
    // PairDecision: 0 = allow, 1 = deny, 2 = no response/dismissed.
    PairDecision(u8),
    // IncomingClipboardDecision: 0 = copy, non-zero = ignore.
    IncomingClipboardDecision(u8),
}

#[uniffi::export(with_foreign)]
pub trait NotificationPlatform: Send + Sync {
    fn show_pair_request(&self, prompt_id: String, device_name: String);
    fn show_pair_code(&self, prompt_id: String, device_name: String, code: String);
    fn show_received_clipboard(&self, prompt_id: String, device_name: String);
    fn notify_info(&self, title: String, message: String);
}

#[uniffi::export(with_foreign)]
pub trait ClipxEventListener: Send + Sync {
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
}
