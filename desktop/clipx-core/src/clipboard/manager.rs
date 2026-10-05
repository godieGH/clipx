use super::clipstore::{ClipItem, ClipKind, ClipboardStore};
use crate::message::proto;
use crate::notification::{
    IncomingClipboardDecision, IncomingClipboardKind, Prompt,
    platform::NotificationEngine as Engine,
};
use crate::platform::ClipboardSink;
use crate::platform::CoreEvent;
use std::{
    collections::{VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const FILE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_RICH_TEXT_BYTES: usize = 8 * 1024 * 1024;
const STAGING_CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Central clipboard processing pipeline for both local and remote content.
///
/// The clipboard manager is responsible for deduplicating changes, storing a
/// history window, prompting the user for remote clipboard approvals, and
/// translating file transfer requests into the correct outbound or inbound
/// actions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FileOfferManifest {
    name: String,
    mime_type: String,
    size: u64,
    expires_at_ms: u64,
}
// const FILE_CHUNK_SIZE: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum ClipboardPayload {
    Text(String),
    RichText {
        text: String,
        html: String,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Files(Vec<String>),
    FileBytes {
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
    FilePath {
        path: PathBuf,
        name: String,
        mime_type: String,
    },
}

/// Outbound instructions emitted by the clipboard manager toward the network or
/// device layers.
#[derive(Debug)]
pub enum ClipboardOutbound {
    Payload(ClipboardPayload),
    FileOffer {
        file_id: String,
        name: String,
        mime_type: String,
        size: u64,
        expires_at_ms: u64,
    },
    FileRequest {
        device_id: String,
        file_id: String,
        entry_id: String,
        file_name: String,
        total_size: u64,
        offer_expires_at_ms: u64,
    },
    FileDownloadFinished {
        file_id: String,
    },
    FileTransferCancel {
        device_id: String,
        file_id: String,
    },
    FileStream {
        device_id: String,
        file_id: String,
        path: PathBuf,
        total_size: u64,
        start_offset: u64,
    },
}

#[allow(unused)]
struct IncomingResolved {
    payload: IncomingPayload,
    source_device_name: String,
    source_device_id: String,
    entry_id: String,
    decision: IncomingClipboardDecision,
}

pub enum IncomingPayload {
    Text(String),
    RichText {
        text: String,
        html: String,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    FileOffer {
        file: proto::FileOffer,
    },
}

/// Commands sent into the clipboard manager's async processing loop.
///
/// These calls describe local history queries, incoming remote content, and file
/// operations that need to be coordinated with the clipboard history store and the
/// user approval flow.
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
    IncomingRemote {
        payload: IncomingPayload,
        source_device_name: String,
        source_device_id: String,
    },
    IncomingFilePath {
        source_device_id: String,
        file_id: String,
        path: PathBuf,
    },
    LocalChangeDetected {
        payload: ClipboardPayload,
    },
    SendLocal {
        payload: ClipboardPayload,
    },
    SendFilePath {
        name: String,
        mime_type: String,
        path: PathBuf,
        reply_to: oneshot::Sender<Result<(), String>>,
    },
    PeerFileDownloadRequest {
        device_id: String,
        file_id: String,
        offset: u64,
    },
    DownloadHistoryFile {
        entry_id: String,
        reply_to: oneshot::Sender<String>,
    },
    FileDownloadReleased {
        file_id: String,
    },
    ResumeDownload {
        entry_id: String,
        file_id: String,
    },
}

/// Runtime coordinator for clipboard content and transfer events.
pub struct ClipboardManager<E: Engine, S: ClipboardSink> {
    store: ClipboardStore,
    notification: E,
    clipboard_sink: S,
    recent_hashes: VecDeque<u64>,
    outbound_tx: mpsc::UnboundedSender<ClipboardOutbound>,
    resolved_tx: mpsc::UnboundedSender<IncomingResolved>,
    resolved_rx: mpsc::UnboundedReceiver<IncomingResolved>,
    events_tx: broadcast::Sender<CoreEvent>,
    files_dir: PathBuf,
    active_downloads: std::collections::HashSet<String>,
}

impl<E: Engine + Clone + 'static, S: ClipboardSink + 'static> ClipboardManager<E, S> {
    pub fn new(
        history_path: PathBuf,
        max_history: usize,
        notification: E,
        clipboard_sink: S,
        outbound_tx: mpsc::UnboundedSender<ClipboardOutbound>,
        events_tx: broadcast::Sender<CoreEvent>,
    ) -> Self {
        let files_dir = crate::device::config::transfer_dir().join("clipboard-files");
        let _ = fs::create_dir_all(&files_dir);
        let store = ClipboardStore::load(history_path, max_history);
        let (resolved_tx, resolved_rx) = mpsc::unbounded_channel();
        Self {
            store,
            notification,
            clipboard_sink,
            recent_hashes: VecDeque::with_capacity(8),
            outbound_tx,
            resolved_tx,
            resolved_rx,
            events_tx,
            files_dir,
            active_downloads: std::collections::HashSet::new(),
        }
    }

    fn notify_clipboard_changed(&self) {
        let _ = self.events_tx.send(CoreEvent::ClipboardChanged);
    }

    fn notify_info(&self, title: &'static str, body: impl Into<String>) {
        let notification = self.notification.clone();
        let body = body.into();
        tokio::spawn(async move {
            notification.notify_info(title, body).await;
        });
    }
    fn notify_transfer(
        &self,
        entry_id: &str,
        file_id: &str,
        file_name: &str,
        direction: &str,
        done: u64,
        total: u64,
        state: &str,
        message: impl Into<String>,
    ) {
        let message = message.into();
        let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: entry_id.to_string(),
            file_id: file_id.to_string(),
            file_name: file_name.to_string(),
            direction: direction.to_string(),
            done,
            total,
            state: state.to_string(),
            message: message.clone(),
        });

        // ClipboardManager owns the post-transfer stages (saving/complete)
        // after DeviceManager has finished receiving the bytes. Forward those
        // stages to the native notification engine too, otherwise the Windows
        // progress toast can remain at 100% with a "Downloading…" status.
        let notification_future = self.notification.notify_file_transfer(
            file_id.to_string(),
            file_name.to_string(),
            direction.to_string(),
            done,
            total,
            state.to_string(),
            message,
            None,
        );
        // Queue the native event before yielding to Tokio. This preserves the
        // state order: requesting -> receiving -> saving -> complete.
        tokio::spawn(notification_future);
    }

    pub async fn run(
        mut self,
        shutdown_rx: watch::Receiver<bool>,
        mut command_rx: mpsc::UnboundedReceiver<ClipboardCommand>,
    ) {
        let (watcher_tx, mut watcher_rx) = mpsc::unbounded_channel();
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        let watcher_task = tokio::spawn(super::watcher::watch_clipboard(
            shutdown_rx.clone(),
            watcher_tx,
        ));
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        drop(watcher_tx);
        let mut shutdown_rx = shutdown_rx;
        let mut cleanup_interval = tokio::time::interval(STAGING_CLEANUP_INTERVAL);
        cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                Some(payload) = watcher_rx.recv() => self.on_local_change(payload),
                Some(cmd) = command_rx.recv() => self.handle_command(cmd),
                Some(resolved) = self.resolved_rx.recv() => self.on_incoming_resolved(resolved),
                _ = cleanup_interval.tick() => self.cleanup_expired_files(),
            }
        }
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        let _ = watcher_task.await;
    }

    fn handle_command(&mut self, cmd: ClipboardCommand) {
        self.cleanup_expired_files();
        match cmd {
            ClipboardCommand::GetHistory { limit, reply_to } => {
                let v = self.history_snapshot();
                let v = v.into_iter().take(limit.unwrap_or(usize::MAX)).collect();
                let _ = reply_to.send(v);
            }
            ClipboardCommand::RemoveEntry { id, reply_to } => {
                let file = self
                    .store
                    .history
                    .iter()
                    .find(|item| item.id == id)
                    .and_then(|item| {
                        item.file_id
                            .as_ref()
                            .map(|file_id| (item.source_device_id.clone(), file_id.clone()))
                    });
                let removed = self.store.remove(&id);
                if removed {
                    if let Some((device_id, file_id)) = file {
                        self.cancel_file_transfer(device_id, file_id);
                    }
                    self.notify_clipboard_changed();
                }
                let _ = reply_to.send(removed);
            }
            ClipboardCommand::ClearHistory { reply_to } => {
                let files: Vec<(String, String)> = self
                    .store
                    .history
                    .iter()
                    .filter_map(|item| {
                        item.file_id
                            .as_ref()
                            .map(|file_id| (item.source_device_id.clone(), file_id.clone()))
                    })
                    .collect();
                let had = !self.store.history.is_empty();
                self.store.clear();
                for (device_id, file_id) in files {
                    self.cancel_file_transfer(device_id, file_id);
                }
                if had {
                    self.notify_clipboard_changed();
                }
                let _ = reply_to.send(());
            }
            ClipboardCommand::IncomingRemote {
                payload,
                source_device_name,
                source_device_id,
            } => self.spawn_incoming_prompt(payload, source_device_name, source_device_id),
            ClipboardCommand::IncomingFilePath {
                source_device_id,
                file_id,
                path,
            } => self.finish_file_download(source_device_id, file_id, path),
            ClipboardCommand::LocalChangeDetected { payload } => self.on_local_change(payload),
            ClipboardCommand::SendLocal { payload } => self.send_local(payload),
            ClipboardCommand::SendFilePath {
                name,
                mime_type,
                path,
                reply_to,
            } => {
                let result = self
                    .store_file_offer_from_path(path, name, mime_type)
                    .map(|_| ());
                let _ = reply_to.send(result);
            }
            ClipboardCommand::PeerFileDownloadRequest {
                device_id,
                file_id,
                offset,
            } => self.send_file_for_peer(device_id, file_id, Some(offset)),
            ClipboardCommand::DownloadHistoryFile { entry_id, reply_to } => {
                let _ = reply_to.send(self.start_download(&entry_id));
            }
            ClipboardCommand::FileDownloadReleased { file_id } => {
                self.release_download(&file_id);
            }
            ClipboardCommand::ResumeDownload { entry_id, file_id } => {
                self.resume_download(&entry_id, &file_id);
            }
        }
    }

    fn send_local(&mut self, payload: ClipboardPayload) {
        if !self.validate_clipboard_payload(&payload) {
            return;
        }
        self.remember_hash(payload_hash(&payload));
        self.emit_local_payload(payload);
    }

    fn on_local_change(&mut self, payload: ClipboardPayload) {
        if !self.validate_clipboard_payload(&payload) {
            return;
        }
        let hash = payload_hash(&payload);
        if self.recent_hashes.contains(&hash) {
            return;
        }
        self.remember_hash(hash);
        self.emit_local_payload(payload);
    }

    fn validate_clipboard_payload(&self, payload: &ClipboardPayload) -> bool {
        match payload {
            ClipboardPayload::Text(text) if text.len() > MAX_CLIPBOARD_TEXT_BYTES => {
                self.notify_info(
                    "Clipboard too large",
                    "This text is larger than 4 MiB. Send it as a file instead.",
                );
                false
            }
            ClipboardPayload::RichText { text, html }
                if text.len() > MAX_CLIPBOARD_TEXT_BYTES
                    || text.len().saturating_add(html.len()) > MAX_RICH_TEXT_BYTES =>
            {
                self.notify_info(
                    "Clipboard too large",
                    "This rich text exceeds the clipboard limit. Send it as a file instead.",
                );
                false
            }
            _ => true,
        }
    }

    fn remember_hash(&mut self, hash: u64) {
        if let Some(pos) = self.recent_hashes.iter().position(|v| *v == hash) {
            self.recent_hashes.remove(pos);
        }
        self.recent_hashes.push_front(hash);
        while self.recent_hashes.len() > 8 {
            self.recent_hashes.pop_back();
        }
    }

    fn emit_local_payload(&mut self, payload: ClipboardPayload) {
        match payload {
            ClipboardPayload::Files(paths) => {
                for path in paths {
                    if let Ok(meta) = fs::metadata(&path) {
                        if !meta.is_file() {
                            continue;
                        }
                        let path_buf = PathBuf::from(&path);
                        let name = path_buf
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("clipboard-file")
                            .to_string();
                        let mime_type = mime_from_name(&name);
                        let _ = self.store_file_offer_from_path(path_buf, name, mime_type);
                    }
                }
            }
            ClipboardPayload::FilePath {
                path,
                name,
                mime_type,
            } => {
                let _ = self.store_file_offer_from_path(path, name, mime_type);
            }
            ClipboardPayload::FileBytes {
                name,
                mime_type,
                data,
            } => {
                let _ = self.store_file_offer_from_bytes(name, mime_type, data);
            }
            ClipboardPayload::Image {
                width,
                height,
                rgba,
            } => {
                let expected = (width as usize)
                    .checked_mul(height as usize)
                    .and_then(|v| v.checked_mul(4));
                if expected != Some(rgba.len())
                    || width == 0
                    || height == 0
                    || rgba.len() > MAX_IMAGE_BYTES
                {
                    tracing::warn!(
                        "clipboard image too large ({} bytes), not auto-sending",
                        rgba.len()
                    );
                    return;
                }
                let _ =
                    self.outbound_tx
                        .send(ClipboardOutbound::Payload(ClipboardPayload::Image {
                            width,
                            height,
                            rgba,
                        }));
            }
            other => {
                let _ = self.outbound_tx.send(ClipboardOutbound::Payload(other));
            }
        }
    }

    fn store_file_offer_from_path(
        &mut self,
        source: PathBuf,
        name: String,
        mime_type: String,
    ) -> Result<String, String> {
        let meta = fs::metadata(&source).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("selected path is not a file".into());
        }
        let file_id = uuid::Uuid::new_v4().to_string();
        let destination = self.files_dir.join(&file_id);
        fs::copy(&source, &destination).map_err(|e| e.to_string())?;
        let expires_at_ms = now_ms() + FILE_TTL_MS;
        let manifest = FileOfferManifest {
            name: name.clone(),
            mime_type: mime_type.clone(),
            size: meta.len(),
            expires_at_ms,
        };
        let manifest_path = self.files_dir.join(format!("{file_id}.meta"));
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if self
            .outbound_tx
            .send(ClipboardOutbound::FileOffer {
                file_id: file_id.clone(),
                name,
                mime_type,
                size: meta.len(),
                expires_at_ms,
            })
            .is_err()
        {
            let _ = fs::remove_file(&destination);
            let _ = fs::remove_file(&manifest_path);
            return Err("device manager is stopped".to_string());
        }
        Ok(file_id)
    }

    fn store_file_offer_from_bytes(
        &mut self,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    ) -> Result<String, String> {
        let file_id = uuid::Uuid::new_v4().to_string();
        let path = self.files_dir.join(&file_id);
        fs::write(&path, &data).map_err(|e| e.to_string())?;
        let expires_at_ms = now_ms() + FILE_TTL_MS;
        let manifest = FileOfferManifest {
            name: name.clone(),
            mime_type: mime_type.clone(),
            size: data.len() as u64,
            expires_at_ms,
        };
        let manifest_path = self.files_dir.join(format!("{file_id}.meta"));
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        if let Err(error) = fs::write(&manifest_path, manifest_bytes) {
            let _ = fs::remove_file(&path);
            return Err(error.to_string());
        }
        if self
            .outbound_tx
            .send(ClipboardOutbound::FileOffer {
                file_id: file_id.clone(),
                name,
                mime_type,
                size: data.len() as u64,
                expires_at_ms,
            })
            .is_err()
        {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(&manifest_path);
            return Err("device manager is stopped".to_string());
        }
        Ok(file_id)
    }

    fn spawn_incoming_prompt(
        &mut self,
        payload: IncomingPayload,
        source_device_name: String,
        source_device_id: String,
    ) {
        match &payload {
            IncomingPayload::Text(text) if text.len() > MAX_CLIPBOARD_TEXT_BYTES => {
                self.notify_info(
                    "Clipboard too large",
                    "Received text is larger than 4 MiB. Send it as a file instead.",
                );
                return;
            }
            IncomingPayload::RichText { text, html }
                if text.len() > MAX_CLIPBOARD_TEXT_BYTES
                    || text.len().saturating_add(html.len()) > MAX_RICH_TEXT_BYTES =>
            {
                self.notify_info(
                    "Clipboard too large",
                    "Received rich text exceeds the clipboard limit. Send it as a file instead.",
                );
                return;
            }
            _ => {}
        }
        // Persist the received item before waiting for the prompt. The prompt is an
        // action on an already-received item, not the gate that determines whether
        // history knows about it. This also makes a dismissed prompt recoverable
        // from history later.
        let item = match &payload {
            IncomingPayload::Text(content) => ClipItem {
                id: uuid::Uuid::new_v4().to_string(),
                content: content.clone(),
                source_device: source_device_name.clone(),
                source_device_id: source_device_id.clone(),
                received_at_ms: now_ms(),
                kind: ClipKind::Text,
                html: None,
                file_id: None,
                file_name: None,
                mime_type: None,
                file_size: 0,
                file_expires_at_ms: 0,
                file_downloaded: false,
                local_file_path: None,
            },
            IncomingPayload::RichText { text, html } => ClipItem {
                id: uuid::Uuid::new_v4().to_string(),
                content: text.clone(),
                source_device: source_device_name.clone(),
                source_device_id: source_device_id.clone(),
                received_at_ms: now_ms(),
                kind: ClipKind::RichText,
                html: Some(html.clone()),
                file_id: None,
                file_name: None,
                mime_type: None,
                file_size: 0,
                file_expires_at_ms: 0,
                file_downloaded: false,
                local_file_path: None,
            },
            IncomingPayload::Image { .. } => ClipItem {
                id: uuid::Uuid::new_v4().to_string(),
                content: "Image clipboard content".into(),
                source_device: source_device_name.clone(),
                source_device_id: source_device_id.clone(),
                received_at_ms: now_ms(),
                kind: ClipKind::Image,
                html: None,
                file_id: None,
                file_name: None,
                mime_type: Some("image/raw".into()),
                file_size: 0,
                file_expires_at_ms: 0,
                file_downloaded: false,
                local_file_path: None,
            },
            IncomingPayload::FileOffer { file } => ClipItem {
                id: uuid::Uuid::new_v4().to_string(),
                content: file.name.clone(),
                source_device: source_device_name.clone(),
                source_device_id: source_device_id.clone(),
                received_at_ms: now_ms(),
                kind: ClipKind::File,
                html: None,
                file_id: Some(file.file_id.clone()),
                file_name: Some(file.name.clone()),
                mime_type: Some(file.mime_type.clone()),
                file_size: file.size,
                file_expires_at_ms: file.expires_at_ms,
                file_downloaded: false,
                local_file_path: None,
            },
        };
        let proposed_entry_id = item.id.clone();
        let added = self.store.add(item);
        let entry_id = if added {
            self.notify_clipboard_changed();
            proposed_entry_id
        } else {
            // ClipboardStore only rejects a duplicate when it is already the
            // history head, so the existing head is the authoritative entry.
            self.store
                .history
                .front()
                .map(|item| item.id.clone())
                .unwrap_or(proposed_entry_id)
        };

        let engine = self.notification.clone();
        let resolved_tx = self.resolved_tx.clone();
        let content = match &payload {
            IncomingPayload::Text(v) => v.clone(),
            IncomingPayload::RichText { text, .. } => text.clone(),
            IncomingPayload::Image { .. } => "Image clipboard content".into(),
            IncomingPayload::FileOffer { file } => format!(
                "{} ({}, {})",
                file.name,
                file.mime_type,
                format_size(file.size)
            ),
        };
        let kind = match &payload {
            IncomingPayload::Text(_) => IncomingClipboardKind::Text,
            IncomingPayload::RichText { .. } => IncomingClipboardKind::RichText,
            IncomingPayload::Image { .. } => IncomingClipboardKind::Image,
            IncomingPayload::FileOffer { .. } => IncomingClipboardKind::File,
        };
        tokio::spawn(async move {
            let decision = engine
                .ask_clipboard(
                    Prompt::IncomingClipboard {
                        peer_name: source_device_name.clone(),
                        content,
                        kind,
                    },
                    Duration::from_secs(30),
                )
                .await;
            let _ = resolved_tx.send(IncomingResolved {
                payload,
                source_device_name,
                source_device_id,
                entry_id,
                decision,
            });
        });
    }

    fn on_incoming_resolved(&mut self, resolved: IncomingResolved) {
        let entry_id = resolved.entry_id;
        if resolved.decision != IncomingClipboardDecision::Copy {
            return;
        }
        match resolved.payload {
            IncomingPayload::Text(content) => {
                self.remember_hash(payload_hash(&ClipboardPayload::Text(content.clone())));
                self.clipboard_sink.write_text(content);
            }
            IncomingPayload::RichText { text, html } => {
                self.remember_hash(payload_hash(&ClipboardPayload::RichText {
                    text: text.clone(),
                    html: html.clone(),
                }));
                self.clipboard_sink.write_rich_text(text, html);
            }
            IncomingPayload::Image {
                width,
                height,
                rgba,
            } => {
                self.remember_hash(payload_hash(&ClipboardPayload::Image {
                    width,
                    height,
                    rgba: rgba.clone(),
                }));
                match encode_png_rgba(width, height, &rgba).and_then(|png| {
                    self.clipboard_sink
                        .save_file("clipx-image.png", "image/png", &png)
                }) {
                    Ok(path) => {
                        if let Some(stored) =
                            self.store.history.iter_mut().find(|i| i.id == entry_id)
                        {
                            stored.kind = ClipKind::File;
                            stored.content = "clipx-image.png".into();
                            stored.file_name = Some("clipx-image.png".into());
                            stored.mime_type = Some("image/png".into());
                            stored.file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            stored.file_downloaded = true;
                            stored.local_file_path = Some(path.clone());
                        }
                        self.store.persist();
                        self.notify_clipboard_changed();
                        let notification = self.notification.clone();
                        tokio::spawn(async move {
                            notification.notify_info("Image saved", path).await;
                        });
                    }
                    Err(e) => {
                        let notification = self.notification.clone();
                        tokio::spawn(async move {
                            notification.notify_info("Image save failed", e).await;
                        });
                    }
                }
            }
            IncomingPayload::FileOffer { .. } => {
                let _ = self.start_download(&entry_id);
            }
        }
    }

    fn finish_file_download(&mut self, source_device_id: String, file_id: String, path: PathBuf) {
        let Some(item) = self
            .store
            .history
            .iter()
            .find(|i| i.file_id.as_deref() == Some(file_id.as_str()))
            .cloned()
        else {
            let _ = fs::remove_file(&path);
            self.release_download(&file_id);
            return;
        };
        let entry_id = item.id.clone();
        if item.source_device_id != source_device_id {
            let _ = fs::remove_file(&path);
            self.notify_transfer(
                &entry_id,
                &file_id,
                item.file_name.as_deref().unwrap_or("file"),
                "download",
                0,
                item.file_size,
                "failed",
                "File data came from an unexpected device",
            );
            self.release_download(&file_id);
            return;
        }
        if item.file_expires_at_ms < now_ms() {
            let _ = fs::remove_file(&path);
            self.notify_transfer(
                &entry_id,
                &file_id,
                item.file_name.as_deref().unwrap_or("file"),
                "download",
                0,
                item.file_size,
                "expired",
                "File offer expired",
            );
            self.release_download(&file_id);
            return;
        }
        let name = item
            .file_name
            .clone()
            .unwrap_or_else(|| "clipx-file".into());
        let mime_type = item
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.notify_transfer(
            &entry_id,
            &file_id,
            &name,
            "download",
            size,
            size,
            "saving",
            format!("Saving {name}"),
        );
        let result = self
            .clipboard_sink
            .save_file_from_path(&name, &mime_type, &path);
        let _ = fs::remove_file(&path);
        match result {
            Ok(path) => {
                let mut updated = item;
                updated.file_downloaded = true;
                updated.local_file_path = Some(path.clone());
                updated.content = name.clone();
                updated.source_device_id = source_device_id;
                let _ = self.store.update(updated);
                self.notify_clipboard_changed();
                self.notify_transfer(
                    &entry_id, &file_id, &name, "download", size, size, "complete", path,
                );
                self.release_download(&file_id);
            }
            Err(e) => {
                self.notify_transfer(&entry_id, &file_id, &name, "download", 0, size, "failed", e);
                self.release_download(&file_id);
            }
        }
    }

    fn start_download(&mut self, entry_id: &str) -> String {
        let Some(item) = self.history_item(entry_id) else {
            return "Clipboard history item not found".into();
        };
        if item.kind != ClipKind::File {
            return "Only file items can be downloaded".into();
        }
        if item.file_downloaded
            && item
                .local_file_path
                .as_deref()
                .is_some_and(|p| PathBuf::from(p).is_file())
        {
            return "File is already downloaded".into();
        }
        if item.file_expires_at_ms < now_ms() {
            return "File offer expired".into();
        }
        let Some(file_id) = item.file_id.clone() else {
            return "File metadata is missing".into();
        };
        if !self.active_downloads.insert(file_id.clone()) {
            return "File download is already in progress".into();
        }
        let file_name = item.file_name.clone().unwrap_or_else(|| "file".to_string());
        if self
            .outbound_tx
            .send(ClipboardOutbound::FileRequest {
                device_id: item.source_device_id,
                file_id: file_id.clone(),
                entry_id: entry_id.to_string(),
                file_name: file_name.clone(),
                total_size: item.file_size,
                offer_expires_at_ms: item.file_expires_at_ms,
            })
            .is_err()
        {
            self.active_downloads.remove(&file_id);
            return "Device manager is stopped".into();
        }
        // DeviceManager owns the actual transfer lifecycle notification.
        // Emitting the same initial state here creates two independent
        // producers, which can race on very small files and briefly reset
        // the UI back to 0%.
        "download requested".into()
    }

    fn resume_download(&mut self, entry_id: &str, file_id: &str) {
        let Some(item) = self.history_item(entry_id) else {
            return;
        };
        if item.kind != ClipKind::File
            || item.file_id.as_deref() != Some(file_id)
            || item.file_downloaded
            || item.file_expires_at_ms < now_ms()
        {
            return;
        }
        self.active_downloads.insert(file_id.to_string());
    }

    fn cancel_file_transfer(&mut self, device_id: String, file_id: String) {
        // History removal is an explicit user cancellation. Tell the device
        // manager first so the active task is cancelled, then immediately
        // remove any sender-side snapshot. Receiver-side cleanup is harmless
        // when no local sender snapshot exists for this file id.
        self.active_downloads.remove(&file_id);
        let _ = self
            .outbound_tx
            .send(ClipboardOutbound::FileTransferCancel {
                device_id: device_id.clone(),
                file_id: file_id.clone(),
            });
        let _ = fs::remove_file(self.files_dir.join(&file_id));
        let _ = fs::remove_file(self.files_dir.join(format!("{file_id}.meta")));

        // Durable cancellation marker: if the process dies after history
        // removal but before DeviceManager handles the cancellation command,
        // startup resume scanning must still treat this transfer as cancelled.
        let incoming_dir = self
            .files_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("incoming-files");
        let _ = fs::create_dir_all(&incoming_dir);
        let marker = incoming_dir.join(format!("clipx-{file_id}.cancelled"));
        let _ = fs::write(&marker, now_ms().to_string());
        let _ = fs::remove_file(incoming_dir.join(format!("clipx-{file_id}.resume")));
        let _ = fs::remove_file(incoming_dir.join(format!("clipx-{device_id}-{file_id}")));
    }

    fn release_download(&mut self, file_id: &str) {
        if self.active_downloads.remove(file_id) {
            let _ = self
                .outbound_tx
                .send(ClipboardOutbound::FileDownloadFinished {
                    file_id: file_id.to_string(),
                });
        }
    }

    fn history_item(&self, id: &str) -> Option<ClipItem> {
        self.store
            .history
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    fn history_snapshot(&mut self) -> Vec<ClipItem> {
        let mut changed = false;
        for item in self.store.history.iter_mut() {
            if item.kind == ClipKind::File && item.file_downloaded {
                let exists = item
                    .local_file_path
                    .as_deref()
                    .is_some_and(|path| PathBuf::from(path).is_file());
                if !exists {
                    item.file_downloaded = false;
                    item.local_file_path = None;
                    changed = true;
                }
            }
        }
        if changed {
            self.store.persist();
            self.notify_clipboard_changed();
        }
        self.store.history.iter().cloned().collect()
    }

    fn cleanup_expired_files(&self) {
        let Ok(entries) = fs::read_dir(&self.files_dir) else {
            return;
        };
        let mut manifests = std::collections::HashSet::new();
        let now = now_ms();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            let file_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<FileOfferManifest>(&bytes) else {
                continue;
            };
            manifests.insert(file_id.clone());
            if manifest.expires_at_ms < now {
                let _ = fs::remove_file(self.files_dir.join(&file_id));
                let _ = fs::remove_file(path);
            }
        }
        // A crash can leave a staged data file without its manifest. The offer
        // TTL is at most 24h, so an orphan older than that cannot represent a
        // live offer and is safe to remove.
        let cutoff = std::time::SystemTime::now()
            .checked_sub(Duration::from_millis(FILE_TTL_MS))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if let Ok(entries) = fs::read_dir(&self.files_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("meta") {
                    continue;
                }
                let id = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if manifests.contains(id) {
                    continue;
                }
                if fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|modified| modified < cutoff)
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    fn send_file_for_peer(
        &mut self,
        device_id: String,
        file_id: String,
        requested_offset: Option<u64>,
    ) {
        let manifest_path = self.files_dir.join(format!("{file_id}.meta"));
        let manifest = match fs::read(&manifest_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<FileOfferManifest>(&b).ok())
        {
            Some(v) => v,
            None => {
                self.notify_info("File unavailable", "The file offer is no longer available");
                return;
            }
        };
        if manifest.expires_at_ms < now_ms() {
            let _ = fs::remove_file(self.files_dir.join(&file_id));
            let _ = fs::remove_file(&manifest_path);
            self.notify_info("File expired", "The file offer has expired");
            return;
        }
        let path = self.files_dir.join(&file_id);
        let Ok(meta) = fs::metadata(&path) else {
            self.notify_info("File unavailable", "The offered file is missing");
            return;
        };
        if meta.len() != manifest.size {
            self.notify_info(
                "File changed",
                "The offered file is no longer the same size",
            );
            return;
        }
        let offset = requested_offset.unwrap_or(0);
        if offset > manifest.size {
            self.notify_info(
                "Resume unavailable",
                "The requested transfer offset is invalid",
            );
            return;
        }
        let _ = self.outbound_tx.send(ClipboardOutbound::FileStream {
            device_id,
            file_id,
            path,
            total_size: manifest.size,
            start_offset: offset,
        });
    }
}

fn payload_hash(p: &ClipboardPayload) -> u64 {
    let mut h = DefaultHasher::new();
    match p {
        ClipboardPayload::Text(s) => {
            0u8.hash(&mut h);
            s.hash(&mut h);
        }
        ClipboardPayload::RichText { text, html } => {
            1u8.hash(&mut h);
            text.hash(&mut h);
            html.hash(&mut h);
        }
        ClipboardPayload::Image {
            width,
            height,
            rgba,
        } => {
            2u8.hash(&mut h);
            width.hash(&mut h);
            height.hash(&mut h);
            rgba.hash(&mut h);
        }
        ClipboardPayload::Files(paths) => {
            3u8.hash(&mut h);
            paths.hash(&mut h);
        }
        ClipboardPayload::FileBytes {
            name,
            mime_type,
            data,
        } => {
            4u8.hash(&mut h);
            name.hash(&mut h);
            mime_type.hash(&mut h);
            data.hash(&mut h);
        }
        ClipboardPayload::FilePath {
            path,
            name,
            mime_type,
        } => {
            5u8.hash(&mut h);
            path.hash(&mut h);
            name.hash(&mut h);
            mime_type.hash(&mut h);
        }
    }
    h.finish()
}

fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or("image dimensions overflow")?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err("invalid RGBA image data".into());
    }

    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("PNG header encode failed: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("PNG image encode failed: {e}"))?;
    drop(writer);
    Ok(encoded)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
fn mime_from_name(name: &str) -> String {
    match std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Best-effort extension for a small set of MIME types clipx commonly
/// transfers. Returns `None` for anything unrecognized rather than
/// guessing — an unrecognized mime type leaves the name untouched.
fn ext_from_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "image/heic" => Some("heic"),
        "application/pdf" => Some("pdf"),
        "text/plain" => Some("txt"),
        "text/html" => Some("html"),
        "application/zip" => Some("zip"),
        "application/json" => Some("json"),
        _ => None,
    }
}

/// Appends the extension implied by `mime_type` when `name` has no
/// extension at all. Never touches a name that already carries *any*
/// extension — even one that doesn't match `mime_type` — so this can never
/// produce a doubled extension like "image.png.png": a name is either
/// missing one or it isn't, and we only act on the former.
pub(crate) fn ensure_file_extension(name: &str, mime_type: &str) -> String {
    let has_extension = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| !e.is_empty());
    if has_extension {
        return name.to_string();
    }
    match ext_from_mime(mime_type) {
        Some(ext) => format!("{name}.{ext}"),
        None => name.to_string(),
    }
}

fn format_size(size: u64) -> String {
    if size >= 1 << 30 {
        format!("{:.1} GB", size as f64 / (1 << 30) as f64)
    } else if size >= 1 << 20 {
        format!("{:.1} MB", size as f64 / (1 << 20) as f64)
    } else if size >= 1 << 10 {
        format!("{:.0} KB", size as f64 / (1 << 10) as f64)
    } else {
        format!("{size} B")
    }
}
