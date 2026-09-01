//! when desktop os don't provide notification engine this stub stands out
//! it ensures other parts of the app on all build target compiles
//! Linux and Macos desktop later will provide a core baked provide a notification engine
//! and gate the module at compile time — the stub then just says non of those three
//! Just as windows does to ensure consistency but this stub still is important for non-windows, 
//! non-Linux, and Non-macos desktops

use super::{platform::NotificationEngine as Engine, IncomingClipboardDecision, PairDecision, Prompt};
use std::time::Duration;

#[derive(Clone)]
pub struct NotificationEngine;

impl NotificationEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Engine for NotificationEngine {

    async fn ask_pair(&self, prompt: Prompt, _timeout: Duration) -> PairDecision {
        tracing::warn!(
            "notification engine not implemented on this platform — auto-denying: {prompt:?}"
        );
        PairDecision::NoResponse
    }

    async fn ask_clipboard(
        &self,
        prompt: Prompt,
        _timeout: Duration,
    ) -> IncomingClipboardDecision {
        tracing::warn!(
            "notification engine not implemented on this platform — auto-ignoring: {prompt:?}"
        );
        IncomingClipboardDecision::Ignore
    }

    // stub_engine.rs
    async fn notify_info(&self, title: &str, body: impl Into<String>) {
        tracing::info!("[{title}] {}", body.into());
    }
}