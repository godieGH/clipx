#[uniffi::export(with_foreign)]
pub trait ClipboardPlatform: Send + Sync {
    fn write_clipboard(&self, content: String);
}

#[derive(uniffi::Enum)]
pub enum NotifierDecision {
    // 0 - allow, 1 - deny, 2 or non 0 and 1 for else(noresponse)
    // pair req and show code both use this
    PairDecision(u8),
    // 0 - allow copy to clipboard non zero to ignore
    IncomingClipboardDecision(u8),

}

#[uniffi::export(with_foreign)]
pub trait NotificationPlatform: Send + Sync {
    fn show_pairing_request_prompt(&self, device_name: String) -> NotifierDecision;
    fn show_pairing_code_prompt(&self, device_name: String, code: String) -> NotifierDecision;
    fn show_received_clipboard_prompt(&self, device_name: String) -> NotifierDecision;

    fn notify_info(&self, title: String, message: String);
}

#[uniffi::export(with_foreign)]
pub trait ClipxEventListener: Send + Sync {
    // this is more like instead of android polling for core to update UI (eg. devices or ect.)
    // android registers this similar callbacks and — when core has something to
    // tells android it calls these and android know its data is stale
    // so it is more like a waking event handling so android sync with the core while the core gets
    // to run some of its components (networking/manage devices etc.) separately from android effects
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
}