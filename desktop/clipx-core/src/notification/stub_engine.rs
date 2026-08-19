use super::{IncomingClipboardDecision, PairDecision, Prompt};
use std::time::Duration;

#[derive(Clone)]
pub struct NotificationEngine;

impl NotificationEngine {
    pub fn new() -> Self {
        Self
    }

    pub async fn ask(&self, prompt: Prompt, _timeout: Duration) -> PairDecision {
        tracing::warn!(
            "notification engine not implemented on this platform — auto-denying: {prompt:?}"
        );
        PairDecision::NoResponse
    }

    pub async fn ask_clipboard(
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
    pub async fn notify_info(&self, title: &str, body: impl Into<String>) {
        tracing::info!("[{title}] {}", body.into());
    }
}