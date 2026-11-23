//! A notification engine that hands prompts to a UI over IPC.
//!
//! The native engines (Windows toasts, nothing at all on Linux and macOS) are a
//! poor place to answer a pairing request or accept a file. This engine lets the
//! desktop app show them itself: a UI connection attaches to the [`PromptHub`],
//! and while one is attached every prompt goes there and is answered there.
//! With no UI attached the engine behaves exactly like the engine it wraps, so
//! prompts are never lost just because the app is closed.
//!
//! Nothing is stored. Prompts live for their timeout, and a UI that attaches
//! late is only shown the ones still waiting.

use super::platform::{
    NotificationEngine as Engine, NotificationFuture, PromptResult, register_pending_prompt,
    remove_pending_prompt, resolve_prompt,
};
use super::{
    IncomingClipboardDecision, IncomingClipboardKind, PairDecision, Prompt,
    TransferNotificationOverride,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

/// Longest preview of incoming content sent to a UI, in characters.
const MAX_CONTENT_PREVIEW_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    PairRequest,
    ConfirmCode,
    IncomingClipboard,
}

/// What a UI needs to show one prompt.
#[derive(Debug, Clone)]
pub struct PromptInfo {
    pub prompt_id: String,
    pub kind: PromptKind,
    pub peer_name: String,
    /// The pairing code to compare, for `ConfirmCode`.
    pub code: String,
    /// A short preview of what arrived, for `IncomingClipboard`.
    pub content: String,
    pub content_kind: IncomingClipboardKind,
    pub timeout_ms: u32,
    pub shown_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedReason {
    Answered,
    TimedOut,
}

/// What the hub tells attached UIs.
#[derive(Debug, Clone)]
pub enum HubEvent {
    Prompt(PromptInfo),
    Closed {
        prompt_id: String,
        reason: ClosedReason,
    },
    /// A message with nothing to answer.
    Info {
        id: String,
        title: String,
        body: String,
        at_ms: u64,
    },
}

/// A user's answer to a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Copy,
    Ignore,
}

struct Shared {
    attached: AtomicUsize,
    pending: Mutex<HashMap<String, PromptInfo>>,
    events: broadcast::Sender<HubEvent>,
}

/// The meeting point between the engine (core side) and attached UIs (IPC side).
#[derive(Clone)]
pub struct PromptHub {
    shared: Arc<Shared>,
}

impl Default for PromptHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps a UI counted as attached for as long as it lives.
pub struct AttachGuard {
    shared: Arc<Shared>,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.shared.attached.fetch_sub(1, Ordering::AcqRel);
    }
}

impl PromptHub {
    pub fn new() -> Self {
        // Large enough that a UI that is briefly slow does not miss a prompt;
        // a UI that does lag can still be caught up from `pending`.
        let (events, _) = broadcast::channel(128);
        Self {
            shared: Arc::new(Shared {
                attached: AtomicUsize::new(0),
                pending: Mutex::new(HashMap::new()),
                events,
            }),
        }
    }

    /// True while at least one UI is attached to show prompts.
    pub fn has_attached_ui(&self) -> bool {
        self.shared.attached.load(Ordering::Acquire) > 0
    }

    /// Attaches a UI. The receiver is subscribed before the pending prompts are
    /// read, so none can fall between them; a prompt may therefore appear in both,
    /// and the caller de-duplicates by `prompt_id`. Pending prompts come oldest first.
    pub fn attach(&self) -> (AttachGuard, broadcast::Receiver<HubEvent>, Vec<PromptInfo>) {
        self.shared.attached.fetch_add(1, Ordering::AcqRel);
        let guard = AttachGuard {
            shared: self.shared.clone(),
        };
        let events = self.shared.events.subscribe();
        (guard, events, self.pending())
    }

    /// Prompts still waiting for an answer, oldest first.
    pub fn pending(&self) -> Vec<PromptInfo> {
        let mut pending: Vec<PromptInfo> = self
            .shared
            .pending
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        pending.sort_by_key(|p| p.shown_at_ms);
        pending
    }

    /// Answers a pending prompt. Fails, leaving the prompt waiting, if it is
    /// unknown (already answered or timed out) or the answer does not fit its
    /// kind, for example `Copy` on a pairing request.
    pub fn resolve(&self, prompt_id: &str, decision: Decision) -> Result<(), String> {
        let mut pending = self.shared.pending.lock().unwrap();
        let Some(info) = pending.get(prompt_id) else {
            return Err("This notification was already answered or has expired".into());
        };
        let result = match (info.kind, decision) {
            (PromptKind::PairRequest | PromptKind::ConfirmCode, Decision::Allow) => {
                PromptResult::Pair(PairDecision::Allow)
            }
            (PromptKind::PairRequest | PromptKind::ConfirmCode, Decision::Deny) => {
                PromptResult::Pair(PairDecision::Deny)
            }
            (PromptKind::IncomingClipboard, Decision::Copy) => {
                PromptResult::Clipboard(IncomingClipboardDecision::Copy)
            }
            (PromptKind::IncomingClipboard, Decision::Ignore) => {
                PromptResult::Clipboard(IncomingClipboardDecision::Ignore)
            }
            _ => return Err("That answer does not fit this notification".into()),
        };
        // Claim it first so a second answer cannot also succeed.
        pending.remove(prompt_id);
        drop(pending);
        resolve_prompt(prompt_id.to_string(), result);
        Ok(())
    }

    fn publish(&self, event: HubEvent) {
        // No receiver just means no UI is listening right now.
        let _ = self.shared.events.send(event);
    }
}

/// Shows prompts in an attached UI, and otherwise defers to `fallback`.
#[derive(Clone)]
pub struct IpcNotificationEngine<F: Engine + Clone> {
    fallback: F,
    hub: PromptHub,
}

impl<F: Engine + Clone> IpcNotificationEngine<F> {
    pub fn new(fallback: F) -> Self {
        Self {
            fallback,
            hub: PromptHub::new(),
        }
    }

    /// The hub UIs attach to.
    pub fn hub(&self) -> PromptHub {
        self.hub.clone()
    }

    /// Publishes the prompt and waits for the answer. `None` means nobody
    /// answered in time.
    async fn ask(&self, mut info: PromptInfo, timeout: Duration) -> Option<PromptResult> {
        let (prompt_id, mut rx) = register_pending_prompt();
        info.prompt_id = prompt_id.clone();
        self.hub
            .shared
            .pending
            .lock()
            .unwrap()
            .insert(prompt_id.clone(), info.clone());
        tracing::debug!(%prompt_id, kind = ?info.kind, timeout_ms = info.timeout_ms, "showing prompt in attached UI");
        self.hub.publish(HubEvent::Prompt(info));

        let waited = tokio::time::timeout(timeout, &mut rx).await;
        remove_pending_prompt(&prompt_id);
        self.hub.shared.pending.lock().unwrap().remove(&prompt_id);
        let result = match waited {
            Ok(Ok(result)) => Some(result),
            // An answer that landed just as the timer fired still counts.
            _ => rx.try_recv().ok(),
        };
        self.hub.publish(HubEvent::Closed {
            prompt_id,
            reason: if result.is_some() {
                ClosedReason::Answered
            } else {
                ClosedReason::TimedOut
            },
        });
        result
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn preview(content: &str) -> String {
    let mut chars = content.chars();
    let head: String = chars.by_ref().take(MAX_CONTENT_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn info_for(prompt: &Prompt, timeout: Duration) -> PromptInfo {
    let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let base = PromptInfo {
        prompt_id: String::new(),
        kind: PromptKind::PairRequest,
        peer_name: String::new(),
        code: String::new(),
        content: String::new(),
        content_kind: IncomingClipboardKind::Text,
        timeout_ms,
        shown_at_ms: now_ms(),
    };
    match prompt {
        Prompt::PairRequest { peer_name } => PromptInfo {
            kind: PromptKind::PairRequest,
            peer_name: peer_name.clone(),
            ..base
        },
        Prompt::ConfirmCode { peer_name, code } => PromptInfo {
            kind: PromptKind::ConfirmCode,
            peer_name: peer_name.clone(),
            code: code.clone(),
            ..base
        },
        Prompt::IncomingClipboard {
            peer_name,
            content,
            kind,
        } => PromptInfo {
            kind: PromptKind::IncomingClipboard,
            peer_name: peer_name.clone(),
            content: preview(content),
            content_kind: *kind,
            ..base
        },
    }
}

impl<F: Engine + Clone + 'static> Engine for IpcNotificationEngine<F> {
    fn ask_pair<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            if !self.hub.has_attached_ui() {
                return self.fallback.ask_pair(prompt, timeout).await;
            }
            if matches!(prompt, Prompt::IncomingClipboard { .. }) {
                return PairDecision::NoResponse;
            }
            match self.ask(info_for(&prompt, timeout), timeout).await {
                Some(PromptResult::Pair(decision)) => decision,
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
            if !self.hub.has_attached_ui() {
                return self.fallback.ask_clipboard(prompt, timeout).await;
            }
            if !matches!(prompt, Prompt::IncomingClipboard { .. }) {
                return IncomingClipboardDecision::Ignore;
            }
            match self.ask(info_for(&prompt, timeout), timeout).await {
                Some(PromptResult::Clipboard(decision)) => decision,
                _ => IncomingClipboardDecision::Ignore,
            }
        })
    }

    fn notify_info<'a>(&'a self, title: &'a str, body: String) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            if !self.hub.has_attached_ui() {
                return self.fallback.notify_info(title, body).await;
            }
            self.hub.publish(HubEvent::Info {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.to_string(),
                body,
                at_ms: now_ms(),
            });
        })
    }

    fn notify_file_transfer(
        &self,
        file_id: String,
        file_name: String,
        direction: String,
        done: u64,
        total: u64,
        state: String,
        message: String,
        override_config: Option<TransferNotificationOverride>,
    ) -> NotificationFuture<'static, ()> {
        // An attached UI follows transfers through the transfer events, so a
        // native progress notification would only duplicate it.
        if self.hub.has_attached_ui() {
            return Box::pin(async {});
        }
        self.fallback.notify_file_transfer(
            file_id,
            file_name,
            direction,
            done,
            total,
            state,
            message,
            override_config,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Counts what reaches the wrapped engine and answers with fixed decisions.
    #[derive(Clone, Default)]
    struct Recorder {
        pairs: Arc<AtomicU32>,
        clipboards: Arc<AtomicU32>,
        infos: Arc<AtomicU32>,
        transfers: Arc<AtomicU32>,
    }

    impl Engine for Recorder {
        fn ask_pair<'a>(&'a self, _: Prompt, _: Duration) -> NotificationFuture<'a, PairDecision> {
            self.pairs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { PairDecision::Deny })
        }
        fn ask_clipboard<'a>(
            &'a self,
            _: Prompt,
            _: Duration,
        ) -> NotificationFuture<'a, IncomingClipboardDecision> {
            self.clipboards.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { IncomingClipboardDecision::Copy })
        }
        fn notify_info<'a>(&'a self, _: &'a str, _: String) -> NotificationFuture<'a, ()> {
            self.infos.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
        fn notify_file_transfer(
            &self,
            _: String,
            _: String,
            _: String,
            _: u64,
            _: u64,
            _: String,
            _: String,
            _: Option<TransferNotificationOverride>,
        ) -> NotificationFuture<'static, ()> {
            self.transfers.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    fn pair_prompt() -> Prompt {
        Prompt::PairRequest {
            peer_name: "Pixel".into(),
        }
    }

    fn file_prompt() -> Prompt {
        Prompt::IncomingClipboard {
            peer_name: "Pixel".into(),
            content: "movie.mp4 (video/mp4, 40 MB)".into(),
            kind: IncomingClipboardKind::File,
        }
    }

    /// Waits for the next prompt event on an attached receiver.
    async fn next_prompt(rx: &mut broadcast::Receiver<HubEvent>) -> PromptInfo {
        loop {
            if let HubEvent::Prompt(info) = rx.recv().await.unwrap() {
                return info;
            }
        }
    }

    #[tokio::test]
    async fn with_no_ui_attached_everything_goes_to_the_wrapped_engine() {
        let fallback = Recorder::default();
        let engine = IpcNotificationEngine::new(fallback.clone());

        assert_eq!(
            engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await,
            PairDecision::Deny
        );
        assert_eq!(
            engine
                .ask_clipboard(file_prompt(), Duration::from_secs(5))
                .await,
            IncomingClipboardDecision::Copy
        );
        engine.notify_info("Hi", "there".into()).await;
        engine
            .notify_file_transfer(
                "f".into(),
                "n".into(),
                "download".into(),
                1,
                2,
                "receiving".into(),
                "".into(),
                None,
            )
            .await;

        for counter in [
            &fallback.pairs,
            &fallback.clipboards,
            &fallback.infos,
            &fallback.transfers,
        ] {
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn an_attached_ui_answers_a_pairing_request() {
        let fallback = Recorder::default();
        let engine = IpcNotificationEngine::new(fallback.clone());
        let hub = engine.hub();
        let (_guard, mut rx, pending) = hub.attach();
        assert!(pending.is_empty());

        let asked = tokio::spawn({
            let engine = engine.clone();
            async move { engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await }
        });
        let shown = next_prompt(&mut rx).await;
        assert_eq!(shown.kind, PromptKind::PairRequest);
        assert_eq!(shown.peer_name, "Pixel");
        assert_eq!(shown.timeout_ms, 5000);

        assert!(hub.resolve(&shown.prompt_id, Decision::Allow).is_ok());
        assert_eq!(asked.await.unwrap(), PairDecision::Allow);

        match rx.recv().await.unwrap() {
            HubEvent::Closed { prompt_id, reason } => {
                assert_eq!(prompt_id, shown.prompt_id);
                assert_eq!(reason, ClosedReason::Answered);
            }
            other => panic!("expected Closed, got {other:?}"),
        }
        assert_eq!(
            fallback.pairs.load(Ordering::SeqCst),
            0,
            "the native engine is not used"
        );
        assert!(hub.attach().2.is_empty(), "nothing is left pending");
    }

    #[tokio::test]
    async fn a_code_prompt_carries_the_code_and_a_file_offer_can_be_downloaded() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();

        let code = tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .ask_pair(
                        Prompt::ConfirmCode {
                            peer_name: "Pixel".into(),
                            code: "482 913".into(),
                        },
                        Duration::from_secs(5),
                    )
                    .await
            }
        });
        let shown = next_prompt(&mut rx).await;
        assert_eq!(
            (shown.kind, shown.code.as_str()),
            (PromptKind::ConfirmCode, "482 913")
        );
        hub.resolve(&shown.prompt_id, Decision::Deny).unwrap();
        assert_eq!(code.await.unwrap(), PairDecision::Deny);

        let file = tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .ask_clipboard(file_prompt(), Duration::from_secs(5))
                    .await
            }
        });
        let shown = next_prompt(&mut rx).await;
        assert_eq!(shown.kind, PromptKind::IncomingClipboard);
        assert_eq!(shown.content_kind, IncomingClipboardKind::File);
        assert!(shown.content.starts_with("movie.mp4"));
        hub.resolve(&shown.prompt_id, Decision::Copy).unwrap();
        assert_eq!(file.await.unwrap(), IncomingClipboardDecision::Copy);
    }

    #[tokio::test]
    async fn an_unanswered_prompt_times_out_and_is_reported_closed() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();

        let asked = tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .ask_pair(pair_prompt(), Duration::from_millis(60))
                    .await
            }
        });
        let shown = next_prompt(&mut rx).await;
        assert_eq!(asked.await.unwrap(), PairDecision::NoResponse);
        match rx.recv().await.unwrap() {
            HubEvent::Closed { prompt_id, reason } => {
                assert_eq!(
                    (prompt_id, reason),
                    (shown.prompt_id.clone(), ClosedReason::TimedOut)
                );
            }
            other => panic!("expected Closed, got {other:?}"),
        }
        // Answering afterwards is refused, not silently accepted.
        assert!(hub.resolve(&shown.prompt_id, Decision::Allow).is_err());
    }

    #[tokio::test]
    async fn an_answer_that_does_not_fit_leaves_the_prompt_waiting() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();
        let asked = tokio::spawn({
            let engine = engine.clone();
            async move { engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await }
        });
        let shown = next_prompt(&mut rx).await;

        assert!(hub.resolve(&shown.prompt_id, Decision::Copy).is_err());
        assert!(hub.resolve("no-such-prompt", Decision::Allow).is_err());
        assert!(
            !asked.is_finished(),
            "the prompt is still waiting for a real answer"
        );
        hub.resolve(&shown.prompt_id, Decision::Deny).unwrap();
        assert_eq!(asked.await.unwrap(), PairDecision::Deny);
    }

    #[tokio::test]
    async fn a_prompt_can_only_be_answered_once() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();
        let asked = tokio::spawn({
            let engine = engine.clone();
            async move { engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await }
        });
        let shown = next_prompt(&mut rx).await;

        assert!(hub.resolve(&shown.prompt_id, Decision::Allow).is_ok());
        assert!(hub.resolve(&shown.prompt_id, Decision::Deny).is_err());
        assert_eq!(
            asked.await.unwrap(),
            PairDecision::Allow,
            "the first answer wins"
        );
    }

    #[tokio::test]
    async fn a_ui_that_attaches_late_still_sees_the_waiting_prompts_oldest_first() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (first_guard, mut first_rx, _) = hub.attach();

        let mut asks = Vec::new();
        for _ in 0..2 {
            asks.push(tokio::spawn({
                let engine = engine.clone();
                async move { engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await }
            }));
            let _ = next_prompt(&mut first_rx).await;
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
        drop(first_guard);

        let (_guard, _rx, pending) = hub.attach();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].shown_at_ms <= pending[1].shown_at_ms);
        for p in pending {
            hub.resolve(&p.prompt_id, Decision::Deny).unwrap();
        }
        for ask in asks {
            assert_eq!(ask.await.unwrap(), PairDecision::Deny);
        }
    }

    #[tokio::test]
    async fn dropping_the_last_ui_sends_prompts_back_to_the_wrapped_engine() {
        let fallback = Recorder::default();
        let engine = IpcNotificationEngine::new(fallback.clone());
        let hub = engine.hub();

        let (first, _rx1, _) = hub.attach();
        let (second, _rx2, _) = hub.attach();
        assert!(hub.has_attached_ui());
        drop(first);
        assert!(hub.has_attached_ui(), "one UI is still attached");
        drop(second);
        assert!(!hub.has_attached_ui());

        assert_eq!(
            engine.ask_pair(pair_prompt(), Duration::from_secs(5)).await,
            PairDecision::Deny
        );
        assert_eq!(fallback.pairs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn info_goes_to_the_ui_and_native_transfer_toasts_are_suppressed_while_attached() {
        let fallback = Recorder::default();
        let engine = IpcNotificationEngine::new(fallback.clone());
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();

        engine
            .notify_info("File expired", "The offer is gone".into())
            .await;
        engine
            .notify_file_transfer(
                "f".into(),
                "n".into(),
                "download".into(),
                1,
                2,
                "receiving".into(),
                "".into(),
                None,
            )
            .await;

        match rx.recv().await.unwrap() {
            HubEvent::Info { title, body, .. } => {
                assert_eq!(
                    (title.as_str(), body.as_str()),
                    ("File expired", "The offer is gone")
                );
            }
            other => panic!("expected Info, got {other:?}"),
        }
        assert_eq!(fallback.infos.load(Ordering::SeqCst), 0);
        assert_eq!(fallback.transfers.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_prompt_of_the_wrong_kind_is_refused_without_bothering_anyone() {
        let engine = IpcNotificationEngine::new(Recorder::default());
        let hub = engine.hub();
        let (_guard, _rx, _) = hub.attach();
        assert_eq!(
            engine.ask_pair(file_prompt(), Duration::from_secs(5)).await,
            PairDecision::NoResponse
        );
        assert_eq!(
            engine
                .ask_clipboard(pair_prompt(), Duration::from_secs(5))
                .await,
            IncomingClipboardDecision::Ignore
        );
        assert!(hub.attach().2.is_empty());
    }

    #[test]
    fn long_content_is_cut_for_the_ui() {
        let long = "é".repeat(MAX_CONTENT_PREVIEW_CHARS + 50);
        let cut = preview(&long);
        assert_eq!(cut.chars().count(), MAX_CONTENT_PREVIEW_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(preview("short"), "short");
    }
}
