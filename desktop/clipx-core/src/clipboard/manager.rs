use super::clipstore::{ClipItem, ClipboardStore};
use crate::message::proto::clipx;
use crate::notification::{platform::NotificationEngine as Engine, IncomingClipboardDecision, Prompt};
use crate::platform::ClipboardSink;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch, broadcast};

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use super::watcher;

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
    /// A host platform reports a local system clipboard change.
    LocalChangeDetected {
        content: String,
    },
}

/// Owns clipboard semantics end-to-end: runs the watcher, applies incoming
/// remote content (after asking the user), and maintains history. Knows
/// nothing about sockets, peers, or transports — it only ever sees plain
/// text in and plain text out.
pub struct ClipboardManager<E: Engine, S: ClipboardSink> {
    store: ClipboardStore,
    notification: E,
    clipboard_sink: S,
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
    /// Pushed to every IPC client whenever history actually changes —
    /// add/remove/clear. Owned by the IPC layer's broadcast channel; this
    /// manager only ever sends into it, never reads from it.
    events_tx: broadcast::Sender<clipx::IpcEvent>,
}

impl<E: Engine + Clone + 'static, S: ClipboardSink + 'static> ClipboardManager<E, S> {
    pub fn new(
        history_path: PathBuf,
        max_history: usize,
        notification: E,
        clipboard_sink: S,
        outbound_tx: mpsc::UnboundedSender<String>,
        events_tx: broadcast::Sender<clipx::IpcEvent>,
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
            clipboard_sink,
            last_known_content,
            outbound_tx,
            resolved_tx,
            resolved_rx,
            events_tx,
        }
    }

    fn notify_clipboard_changed(&self) {
        let _ = self.events_tx.send(clipx::IpcEvent {
            event: Some(clipx::ipc_event::Event::ClipboardChanged(
                clipx::ClipboardChangedEvent {},
            )),
        });
    }

    /// Owns the watcher task's lifetime internally — nothing outside this
    /// module needs to know a poller exists, let alone wire it up.
    pub async fn run(
        mut self,
        shutdown_rx: watch::Receiver<bool>,
        mut command_rx: mpsc::UnboundedReceiver<ClipboardCommand>,
    ) {
        let (watcher_tx, mut watcher_rx) = mpsc::unbounded_channel();

        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        let watcher_task = tokio::spawn(watcher::watch_clipboard(shutdown_rx.clone(), watcher_tx));

        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        drop(watcher_tx);

        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                Some(text) = watcher_rx.recv() => { self.on_local_change(text); }
                Some(cmd) = command_rx.recv() => { self.handle_command(cmd); }
                Some(resolved) = self.resolved_rx.recv() => { self.on_incoming_resolved(resolved); }
            }
        }
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
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
                let removed = self.store.remove(&id);
                if removed {
                    self.notify_clipboard_changed();
                }
                let _ = reply_to.send(removed);
            }
            ClipboardCommand::ClearHistory { reply_to } => {
                let had_items = !self.store.history.is_empty();
                self.store.clear();
                if had_items {
                    self.notify_clipboard_changed();
                }
                let _ = reply_to.send(());
            }
            ClipboardCommand::IncomingRemote {
                content,
                source_device_name,
            } => self.spawn_incoming_prompt(content, source_device_name),
            ClipboardCommand::LocalChangeDetected { content } => self.on_local_change(content),
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

        let inserted = self.store.add(ClipItem {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.clone(),
            source_device: source_device_name,
            received_at_ms: now_ms(),
        });
        if inserted {
            self.notify_clipboard_changed();
        }

        if decision == IncomingClipboardDecision::Copy {
            self.last_known_content = content.clone();
            self.apply_to_system_clipboard(&content);
        }
    }

    fn apply_to_system_clipboard(&self, content: &str) {
        self.clipboard_sink.write(content.to_string());
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
