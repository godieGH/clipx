use std::sync::Arc;

use crate::platform::NotificationPrompter;

use super::{PairDecision, IncomingClipboardDecision, Prompt};
use std::time::Duration;

#[allow(async_fn_in_trait)]
pub trait NotificationEngine {

    async fn ask_pair(&self, prompt: Prompt, timeout: Duration) -> PairDecision;

    async fn ask_clipboard(
        &self,
        prompt: Prompt,
        timeout: Duration,
    ) -> IncomingClipboardDecision;

    async fn notify_info(&self, title: &str, body: impl Into<String>);
}


/// core platform notification engine wrapper
/// it wraps any object that implements the notification prompter
pub struct PlatformNotificationEngine<T: NotificationPrompter + ?Sized> {
    notifier: Arc<T>
}

impl<T: NotificationPrompter + ?Sized> PlatformNotificationEngine<T> {
    pub fn new(notifier: Arc<T>) -> Self {
        Self {
            notifier
        }
    }
}

impl<T: NotificationPrompter + ?Sized> NotificationEngine for PlatformNotificationEngine<T> {
    /// for now ignore timeout until we have something structural
    /// because even now this is not synchronous maybe because of platform boundary 
    /// nature — it is not like window were you register a com callback this is different
    /// maybe we need to look into it
    /// or we can later think of using tokio::task::spawn_blocking and await it as normal 
    /// now the core can continue doing things not blocked while after finish we look what the 
    /// joinable handle has returned for us — but for now let us just make things standout structured
    async fn ask_pair(&self, prompt: Prompt, _timeout: Duration) -> PairDecision {
        match prompt {
            Prompt::PairRequest { peer_name } => {
                self.notifier.ask_pair_request(peer_name)
            }
            Prompt::ConfirmCode { peer_name, code } => {
                self.notifier.ask_pair_code(peer_name, code)
            }
            _ => PairDecision::NoResponse
        }
    }

    async fn ask_clipboard(
        &self,
        prompt: Prompt,
        _timeout: Duration,
    ) -> IncomingClipboardDecision
    {
        match prompt {
            // content doesn't pass platform
            Prompt::IncomingClipboard { peer_name, ..} => {
                self.notifier.ask_received_clipboard(peer_name)
            }
            _ => IncomingClipboardDecision::Ignore
        }
    }

    /// maybe this one doesn't need waiting since it is a fire and forget
    async fn notify_info(&self, title: &str, body: impl Into<String>) {
        self.notifier.notify_info(title.into(), body.into());
    }
}