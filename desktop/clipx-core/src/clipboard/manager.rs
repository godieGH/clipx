use super::clipstore::{ClipItem, ClipboardStore};
use super::watcher;
use crate::notification::{IncomingClipboardDecision, NotificationEngine, Prompt};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch};

/// Result of a toast fed back into the manager's own select loop.
/// `NotificationEngine::ask_clipboard()` can take up to its timeout, so
/// it's spawned as its own task rather than awaited inline — awaiting it
/// directly would stall the watcher and every other clipboard command for
/// as long as the toast is on screen.
struct IncomingResolved {
    content: String,
    source_device_name: String,
    decision: IncomingClipboardDecision,
}

/// Everything outside the clipboard module — Device Manager, IPC layer —
/// talks to the clipboard through this. It's the only door in.
pub enum ClipboardCommand {
    GetHistory {
        limit: Option<usize>,
        reply_to: oneshot::Sender<Vec<ClipItem>>,
    },
    RemoveEntry {
        id: String,
        reply_to: oneshot::Sender<bool>,
    },
    ClearHistory {
        reply_to: oneshot::Sender<()>,
    },
    /// Device Manager delivers this once it has decoded a ClipboardMessage
    /// from a *trusted* peer and resolved a display name for it. Whether
    /// that content gets prompted, applied, and/or recorded is entirely
    /// this manager's call — Device Manager has no say past this point.
    IncomingRemote {
        content: String,
        source_device_name: String,
    },
}

/// Owns clipboard semantics end-to-end: runs the watcher, applies incoming
/// remote content (after asking the user), and maintains history. Knows
/// nothing about sockets, peers, or transports — it only ever sees plain
/// text in and plain text out.
pub struct ClipboardManager {
    store: ClipboardStore,
    notification: NotificationEngine,
    /// Mirrors what we believe the OS clipboard currently holds. Sharing
    /// this single value between "watcher detected a change" and "we just
    /// applied a remote value" is what stops an applied remote update from
    /// being picked back up by the watcher and re-broadcast as if it were
    /// a new local change.
    last_known_content: String,
    /// Local clipboard changes to hand to the Device Manager for dispatch.
    /// This manager doesn't know or care who receives them.
    outbound_tx: mpsc::UnboundedSender<String>,
    resolved_tx: mpsc::UnboundedSender<IncomingResolved>,
    resolved_rx: mpsc::UnboundedReceiver<IncomingResolved>,
}

impl ClipboardManager {
    pub fn new(
        history_path: PathBuf,
        max_history: usize,
        notification: NotificationEngine,
        outbound_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        let store = ClipboardStore::load(history_path, max_history);
        let last_known_content = store
            .history
            .front()
            .map(|i| i.content.clone())
            .unwrap_or_default();
        let (resolved_tx, resolved_rx) = mpsc::unbounded_channel();
        Self {
            store,
            notification,
            last_known_content,
            outbound_tx,
            resolved_tx,
            resolved_rx,
        }
    }

    /// Owns the watcher task's lifetime internally — nothing outside this
    /// module needs to know a poller exists, let alone wire it up.
    pub async fn run(
        mut self,
        shutdown_rx: watch::Receiver<bool>,
        mut command_rx: mpsc::UnboundedReceiver<ClipboardCommand>,
    ) {
        let (watcher_tx, mut watcher_rx) = mpsc::unbounded_channel();
        let watcher_task = tokio::spawn(watcher::watch_clipboard(shutdown_rx.clone(), watcher_tx));

        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                Some(text) = watcher_rx.recv() => { self.on_local_change(text); }
                Some(cmd) = command_rx.recv() => { self.handle_command(cmd); }
                Some(resolved) = self.resolved_rx.recv() => { self.on_incoming_resolved(resolved); }
            }
        }
        let _ = watcher_task.await;
        tracing::info!("clipboard manager stopped");
    }

    fn handle_command(&mut self, cmd: ClipboardCommand) {
        match cmd {
            ClipboardCommand::GetHistory { limit, reply_to } => {
                let items: Vec<ClipItem> = match limit {
                    Some(n) => self.store.history.iter().take(n).cloned().collect(),
                    None => self.store.history.iter().cloned().collect(),
                };
                let _ = reply_to.send(items);
            }
            ClipboardCommand::RemoveEntry { id, reply_to } => {
                let _ = reply_to.send(self.store.remove(&id));
            }
            ClipboardCommand::ClearHistory { reply_to } => {
                self.store.clear();
                let _ = reply_to.send(());
            }
            ClipboardCommand::IncomingRemote {
                content,
                source_device_name,
            } => self.spawn_incoming_prompt(content, source_device_name),
        }
    }

    /// Called when the watcher detects the OS clipboard changed. History
    /// only ever holds what *other* devices sent you — a local copy just
    /// updates the echo guard and goes straight out to be dispatched.
    fn on_local_change(&mut self, text: String) {
        if text == self.last_known_content {
            // Echo of content we just wrote to the OS clipboard ourselves
            // (from on_incoming_resolved below) — not a new local change.
            return;
        }
        self.last_known_content = text.clone();
        if self.outbound_tx.send(text).is_err() {
            tracing::warn!("no listener for outbound clipboard changes (device manager down?)");
        }
    }

    /// Content arrived from a paired device. Unlike a local change, writing
    /// someone else's clipboard into yours is a decision the user should
    /// get to make — so this asks first via the notification engine. The
    /// toast is spawned off (see `IncomingResolved`) rather than awaited
    /// here so the watcher and other commands keep flowing while it's up.
    fn spawn_incoming_prompt(&mut self, content: String, source_device_name: String) {
        let engine = self.notification.clone();
        let resolved_tx = self.resolved_tx.clone();
        let prompt_content = content.clone();
        let prompt_name = source_device_name.clone();
        tokio::spawn(async move {
            let decision = engine
                .ask_clipboard(
                    Prompt::IncomingClipboard {
                        peer_name: prompt_name,
                        content: prompt_content,
                    },
                    Duration::from_secs(30),
                )
                .await;
            let _ = resolved_tx.send(IncomingResolved {
                content,
                source_device_name,
                decision,
            });
        });
    }

    /// Recorded to history regardless of the user's answer, so they can
    /// still grab it from the history UI even after dismissing/ignoring
    /// the prompt — only *applying* it to the live clipboard is gated.
    fn on_incoming_resolved(&mut self, resolved: IncomingResolved) {
        let IncomingResolved {
            content,
            source_device_name,
            decision,
        } = resolved;

        self.store.add(ClipItem {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.clone(),
            source_device: source_device_name,
            received_at_ms: now_ms(),
        });

        if decision == IncomingClipboardDecision::Copy {
            self.last_known_content = content.clone();
            self.apply_to_system_clipboard(&content);
        }
    }

    #[cfg(not(target_os = "android"))]
    fn apply_to_system_clipboard(&self, content: &str) {
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(content.to_string()) {
                    tracing::warn!("failed to apply remote clipboard content: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to open clipboard to apply remote content: {e}"),
        }
    }

    #[cfg(target_os = "android")]
    fn apply_to_system_clipboard(&self, _content: &str) {
        // Android clipboard access goes through the platform module, not arboard.
        tracing::info!("apply_to_system_clipboard is not implemented for Android in this crate");
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
