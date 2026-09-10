use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex},
    collections::HashMap,
    time::Duration,
};

use crate::platform::NotificationPrompter;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{IncomingClipboardDecision, IncomingClipboardKind, PairDecision, Prompt};

pub type NotificationFuture<'a, T> =
    Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait NotificationEngine: Send + Sync {
    fn ask_pair<'a>(&'a self, prompt: Prompt, timeout: Duration) -> NotificationFuture<'a, PairDecision>;

    fn ask_clipboard<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision>;

    fn notify_info<'a>(
        &'a self,
        title: &'a str,
        body: String,
    ) -> NotificationFuture<'a, ()>;

    /// Optional native transfer notification. Platforms without a native
    /// progress notification simply inherit this no-op implementation.
    fn notify_file_transfer<'a>(
        &'a self,
        _file_id: String,
        _file_name: String,
        _direction: String,
        _done: u64,
        _total: u64,
        _state: String,
        _message: String,
    ) -> NotificationFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PromptResult {
    Pair(PairDecision),
    Clipboard(IncomingClipboardDecision),
}

static PENDING_PROMPTS: LazyLock<Mutex<HashMap<String, oneshot::Sender<PromptResult>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
pub struct PlatformNotificationEngine<T: NotificationPrompter> {
    notifier: Arc<T>,
}

impl<T: NotificationPrompter> PlatformNotificationEngine<T> {
    pub fn new(notifier: Arc<T>) -> Self {
        Self { notifier }
    }

    fn register_prompt() -> (String, oneshot::Receiver<PromptResult>) {
        let prompt_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        PENDING_PROMPTS.lock().unwrap().insert(prompt_id.clone(), tx);
        (prompt_id, rx)
    }

    fn remove_prompt(prompt_id: &str) {
        PENDING_PROMPTS.lock().unwrap().remove(prompt_id);
    }
}

pub fn resolve_prompt(prompt_id: String, result: PromptResult) {
    let sender = PENDING_PROMPTS.lock().unwrap().remove(&prompt_id);
    if let Some(sender) = sender {
        let _ = sender.send(result);
    } else {
        tracing::debug!(%prompt_id, "notification prompt was already resolved or timed out");
    }
}

impl<T: NotificationPrompter> NotificationEngine for PlatformNotificationEngine<T> {
    fn ask_pair<'a>(&'a self, prompt: Prompt, timeout: Duration) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            let (prompt_id, rx) = Self::register_prompt();

            match prompt {
                Prompt::PairRequest { peer_name } => {
                    self.notifier.show_pair_request(prompt_id.clone(), peer_name);
                }
                Prompt::ConfirmCode { peer_name, code } => {
                    self.notifier.show_pair_code(prompt_id.clone(), peer_name, code);
                }
                _ => {
                    Self::remove_prompt(&prompt_id);
                    return PairDecision::NoResponse;
                }
            }

            let result = tokio::time::timeout(timeout, rx).await;
            Self::remove_prompt(&prompt_id);
            match result {
                Ok(Ok(PromptResult::Pair(decision))) => decision,
                _ => PairDecision::NoResponse,
            }
        })
    }

    fn ask_clipboard<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision> {
        Box::pin(async move {
            let (prompt_id, rx) = Self::register_prompt();

            match prompt {
                Prompt::IncomingClipboard { peer_name, kind, .. } => {
                    let action = match kind {
                        IncomingClipboardKind::Image => "Save image",
                        IncomingClipboardKind::File => "Download file",
                        IncomingClipboardKind::Text | IncomingClipboardKind::RichText => "Copy to clipboard",
                    };
                    self.notifier.show_received_clipboard(prompt_id.clone(), peer_name, action.to_string());
                }
                _ => {
                    Self::remove_prompt(&prompt_id);
                    return IncomingClipboardDecision::Ignore;
                }
            }

            let result = tokio::time::timeout(timeout, rx).await;
            Self::remove_prompt(&prompt_id);
            match result {
                Ok(Ok(PromptResult::Clipboard(decision))) => decision,
                _ => IncomingClipboardDecision::Ignore,
            }
        })
    }

    fn notify_info<'a>(&'a self, title: &'a str, body: String) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            self.notifier.notify_info(title.to_string(), body);
        })
    }
}
