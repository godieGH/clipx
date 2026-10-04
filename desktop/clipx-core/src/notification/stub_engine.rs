//! when desktop os don't provide notification engine this stub stands out
//! it ensures other parts of the app on all build target compiles
//! Linux and Macos desktop later will provide a core baked provide a notification engine
//! and gate the module at compile time — the stub then just says non of those three
//! Just as windows does to ensure consistency but this stub still is important for non-windows,
//! non-Linux, and Non-macos desktops

use super::{
    IncomingClipboardDecision, NotificationConfig, PairDecision, Prompt,
    TransferNotificationOverride,
    platform::{NotificationEngine as Engine, NotificationFuture},
};
use std::time::Duration;

#[derive(Clone)]
pub struct NotificationEngine;

impl NotificationEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn with_config(_config: NotificationConfig) -> Self {
        Self::new()
    }
}

impl Engine for NotificationEngine {
    fn ask_pair<'a>(
        &'a self,
        prompt: Prompt,
        _timeout: Duration,
    ) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            tracing::warn!(
                "notification engine not implemented on this platform — auto-denying: {prompt:?}"
            );
            PairDecision::NoResponse
        })
    }

    fn ask_clipboard<'a>(
        &'a self,
        prompt: Prompt,
        _timeout: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision> {
        Box::pin(async move {
            tracing::warn!(
                "notification engine not implemented on this platform — auto-ignoring: {prompt:?}"
            );
            IncomingClipboardDecision::Ignore
        })
    }

    fn notify_info<'a>(&'a self, title: &'a str, body: String) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            tracing::info!("[{title}] {body}");
        })
    }

    fn notify_file_transfer(
        &self,
        _file_id: String,
        _file_name: String,
        _direction: String,
        _done: u64,
        _total: u64,
        _state: String,
        _message: String,
        _override: Option<TransferNotificationOverride>,
    ) -> NotificationFuture<'static, ()> {
        Box::pin(async {})
    }
}
