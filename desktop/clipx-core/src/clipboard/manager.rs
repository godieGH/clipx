use super::clipstore::{ClipItem, ClipKind, ClipboardStore};
use crate::message::proto;
use crate::notification::{platform::NotificationEngine as Engine, IncomingClipboardDecision, IncomingClipboardKind, Prompt};
use crate::platform::ClipboardSink;
use crate::platform::CoreEvent;
use std::{collections::{hash_map::DefaultHasher, VecDeque}, fs, hash::{Hash, Hasher}, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const FILE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

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
    RichText { text: String, html: String },
    Image { width: u32, height: u32, rgba: Vec<u8> },
    Files(Vec<String>),
    FileBytes { name: String, mime_type: String, data: Vec<u8> },
    FilePath { path: PathBuf, name: String, mime_type: String },
}

#[derive(Debug)]
pub enum ClipboardOutbound {
    Payload(ClipboardPayload),
    FileOffer { file_id: String, name: String, mime_type: String, size: u64, expires_at_ms: u64 },
    FileRequest { device_id: String, file_id: String, entry_id: String },
    FileStream { device_id: String, file_id: String, path: PathBuf, total_size: u64 },
}

struct IncomingResolved {
    payload: IncomingPayload,
    source_device_name: String,
    source_device_id: String,
    decision: IncomingClipboardDecision,
}

pub enum IncomingPayload { Text(String), RichText { text: String, html: String }, Image { width: u32, height: u32, rgba: Vec<u8> }, FileOffer { file: proto::FileOffer } }

pub enum ClipboardCommand {
    GetHistory { limit: Option<usize>, reply_to: oneshot::Sender<Vec<ClipItem>> },
    RemoveEntry { id: String, reply_to: oneshot::Sender<bool> },
    ClearHistory { reply_to: oneshot::Sender<()> },
    IncomingRemote { payload: IncomingPayload, source_device_name: String, source_device_id: String },
    IncomingFilePath { source_device_id: String, file_id: String, path: PathBuf },
    LocalChangeDetected { payload: ClipboardPayload },
    SendLocal { payload: ClipboardPayload },
    SendFilePath { name: String, mime_type: String, path: PathBuf, reply_to: oneshot::Sender<Result<(), String>> },
    PeerFileDownloadRequest { device_id: String, file_id: String },
    DownloadHistoryFile { entry_id: String, reply_to: oneshot::Sender<String> },
}

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
}

impl<E: Engine + Clone + 'static, S: ClipboardSink + 'static> ClipboardManager<E, S> {
    pub fn new(history_path: PathBuf, max_history: usize, notification: E, clipboard_sink: S, outbound_tx: mpsc::UnboundedSender<ClipboardOutbound>, events_tx: broadcast::Sender<CoreEvent>) -> Self {
        let files_dir = history_path.parent().unwrap_or_else(|| std::path::Path::new(".")).join("clipboard-files");
        let _ = fs::create_dir_all(&files_dir);
        let store = ClipboardStore::load(history_path, max_history);
        let (resolved_tx, resolved_rx) = mpsc::unbounded_channel();
        Self { store, notification, clipboard_sink, recent_hashes: VecDeque::with_capacity(8), outbound_tx, resolved_tx, resolved_rx, events_tx, files_dir }
    }


    fn notify_clipboard_changed(&self) { let _ = self.events_tx.send(CoreEvent::ClipboardChanged); }

    fn notify_info(&self, title: &'static str, body: impl Into<String>) {
        let notification = self.notification.clone();
        let body = body.into();
        tokio::spawn(async move { notification.notify_info(title, body).await; });
    }
    fn notify_transfer(&self, entry_id: &str, file_id: &str, done: u64, total: u64, state: &str, message: impl Into<String>) {
        let _ = self.events_tx.send(CoreEvent::FileTransferChanged { entry_id: entry_id.to_string(), file_id: file_id.to_string(), done, total, state: state.to_string(), message: message.into() });
    }

    pub async fn run(mut self, shutdown_rx: watch::Receiver<bool>, mut command_rx: mpsc::UnboundedReceiver<ClipboardCommand>) {
        let (watcher_tx, mut watcher_rx) = mpsc::unbounded_channel();
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        let watcher_task = tokio::spawn(super::watcher::watch_clipboard(shutdown_rx.clone(), watcher_tx));
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        drop(watcher_tx);
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
                Some(payload) = watcher_rx.recv() => self.on_local_change(payload),
                Some(cmd) = command_rx.recv() => self.handle_command(cmd),
                Some(resolved) = self.resolved_rx.recv() => self.on_incoming_resolved(resolved),
            }
        }
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        let _ = watcher_task.await;
    }

    fn handle_command(&mut self, cmd: ClipboardCommand) {
        self.cleanup_expired_files();
        match cmd {
            ClipboardCommand::GetHistory { limit, reply_to } => { let v = self.store.history.iter().take(limit.unwrap_or(usize::MAX)).cloned().collect(); let _ = reply_to.send(v); }
            ClipboardCommand::RemoveEntry { id, reply_to } => { let v = self.store.remove(&id); if v { self.notify_clipboard_changed(); } let _ = reply_to.send(v); }
            ClipboardCommand::ClearHistory { reply_to } => { let had = !self.store.history.is_empty(); self.store.clear(); if had { self.notify_clipboard_changed(); } let _ = reply_to.send(()); }
            ClipboardCommand::IncomingRemote { payload, source_device_name, source_device_id } => self.spawn_incoming_prompt(payload, source_device_name, source_device_id),
            ClipboardCommand::IncomingFilePath { source_device_id, file_id, path } => self.finish_file_download(source_device_id, file_id, path),
            ClipboardCommand::LocalChangeDetected { payload } => self.on_local_change(payload),
            ClipboardCommand::SendLocal { payload } => self.send_local(payload),
            ClipboardCommand::SendFilePath { name, mime_type, path, reply_to } => {
                let result = self.store_file_offer_from_path(path, name, mime_type).map(|_| ());
                let _ = reply_to.send(result);
            }
            ClipboardCommand::PeerFileDownloadRequest { device_id, file_id } => self.send_file_for_peer(device_id, file_id),
            ClipboardCommand::DownloadHistoryFile { entry_id, reply_to } => { let _ = reply_to.send(self.start_download(&entry_id)); },
        }
    }

    fn send_local(&mut self, payload: ClipboardPayload) {
        self.remember_hash(payload_hash(&payload));
        self.emit_local_payload(payload);
    }

    fn on_local_change(&mut self, payload: ClipboardPayload) {
        let hash = payload_hash(&payload);
        if self.recent_hashes.contains(&hash) { return; }
        self.remember_hash(hash);
        self.emit_local_payload(payload);
    }

    fn remember_hash(&mut self, hash: u64) {
        if let Some(pos) = self.recent_hashes.iter().position(|v| *v == hash) { self.recent_hashes.remove(pos); }
        self.recent_hashes.push_front(hash);
        while self.recent_hashes.len() > 8 { self.recent_hashes.pop_back(); }
    }

    fn emit_local_payload(&mut self, payload: ClipboardPayload) {
        match payload {
            ClipboardPayload::Files(paths) => {
                for path in paths {
                    if let Ok(meta) = fs::metadata(&path) {
                        if !meta.is_file() { continue; }
                        let path_buf = PathBuf::from(&path);
                        let name = path_buf.file_name().and_then(|s| s.to_str()).unwrap_or("clipboard-file").to_string();
                        let mime_type = mime_from_name(&name);
                        let _ = self.store_file_offer_from_path(path_buf, name, mime_type);
                    }
                }
            }
            ClipboardPayload::FilePath { path, name, mime_type } => { let _ = self.store_file_offer_from_path(path, name, mime_type); }
            ClipboardPayload::FileBytes { name, mime_type, data } => { let _ = self.store_file_offer_from_bytes(name, mime_type, data); }
            ClipboardPayload::Image { width, height, rgba } => {
                let expected = (width as usize).checked_mul(height as usize).and_then(|v| v.checked_mul(4));
                if expected != Some(rgba.len()) || width == 0 || height == 0 || rgba.len() > MAX_IMAGE_BYTES {
                    tracing::warn!("clipboard image too large ({} bytes), not auto-sending", rgba.len());
                    return;
                }
                let _ = self.outbound_tx.send(ClipboardOutbound::Payload(ClipboardPayload::Image { width, height, rgba }));
            }
            other => { let _ = self.outbound_tx.send(ClipboardOutbound::Payload(other)); }
        }
    }

    fn store_file_offer_from_path(&mut self, source: PathBuf, name: String, mime_type: String) -> Result<String, String> {
        let meta = fs::metadata(&source).map_err(|e| e.to_string())?;
        if !meta.is_file() { return Err("selected path is not a file".into()); }
        let file_id = uuid::Uuid::new_v4().to_string();
        let destination = self.files_dir.join(&file_id);
        if fs::hard_link(&source, &destination).is_err() {
            fs::copy(&source, &destination).map_err(|e| e.to_string())?;
        }
        let expires_at_ms = now_ms() + FILE_TTL_MS;
        let manifest = FileOfferManifest { name: name.clone(), mime_type: mime_type.clone(), size: meta.len(), expires_at_ms };
        let manifest_path = self.files_dir.join(format!("{file_id}.meta"));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if self.outbound_tx.send(ClipboardOutbound::FileOffer { file_id: file_id.clone(), name, mime_type, size: meta.len(), expires_at_ms }).is_err() {
            let _ = fs::remove_file(&destination);
            let _ = fs::remove_file(&manifest_path);
            return Err("device manager is stopped".to_string());
        }
        Ok(file_id)
    }

    fn store_file_offer_from_bytes(&mut self, name: String, mime_type: String, data: Vec<u8>) -> Result<String, String> {
        let file_id = uuid::Uuid::new_v4().to_string();
        let path = self.files_dir.join(&file_id);
        fs::write(&path, &data).map_err(|e| e.to_string())?;
        let expires_at_ms = now_ms() + FILE_TTL_MS;
        let manifest = FileOfferManifest { name: name.clone(), mime_type: mime_type.clone(), size: data.len() as u64, expires_at_ms };
        fs::write(self.files_dir.join(format!("{file_id}.meta")), serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        self.outbound_tx.send(ClipboardOutbound::FileOffer { file_id: file_id.clone(), name, mime_type, size: data.len() as u64, expires_at_ms }).map_err(|_| "device manager is stopped".to_string())?;
        Ok(file_id)
    }

    fn spawn_incoming_prompt(&mut self, payload: IncomingPayload, source_device_name: String, source_device_id: String) {
        let engine = self.notification.clone();
        let resolved_tx = self.resolved_tx.clone();
        let content = match &payload { IncomingPayload::Text(v) => v.clone(), IncomingPayload::RichText { text, .. } => text.clone(), IncomingPayload::Image { .. } => "Image clipboard content".into(), IncomingPayload::FileOffer { file } => format!("{} ({}, {})", file.name, file.mime_type, format_size(file.size)) };
        tokio::spawn(async move {
            let kind = match &payload {
                IncomingPayload::Text(_) => IncomingClipboardKind::Text,
                IncomingPayload::RichText { .. } => IncomingClipboardKind::RichText,
                IncomingPayload::Image { .. } => IncomingClipboardKind::Image,
                IncomingPayload::FileOffer { .. } => IncomingClipboardKind::File,
            };
            let decision = engine.ask_clipboard(Prompt::IncomingClipboard { peer_name: source_device_name.clone(), content: content.clone(), kind }, Duration::from_secs(30)).await;
            let _ = resolved_tx.send(IncomingResolved { payload, source_device_name, source_device_id, decision });
        });
    }

    fn on_incoming_resolved(&mut self, resolved: IncomingResolved) {
        let item = match &resolved.payload {
            IncomingPayload::Text(content) => ClipItem { id: uuid::Uuid::new_v4().to_string(), content: content.clone(), source_device: resolved.source_device_name.clone(), source_device_id: resolved.source_device_id.clone(), received_at_ms: now_ms(), kind: ClipKind::Text, html: None, file_id: None, file_name: None, mime_type: None, file_size: 0, file_expires_at_ms: 0, file_downloaded: false, local_file_path: None },
            IncomingPayload::RichText { text, html } => ClipItem { id: uuid::Uuid::new_v4().to_string(), content: text.clone(), source_device: resolved.source_device_name.clone(), source_device_id: resolved.source_device_id.clone(), received_at_ms: now_ms(), kind: ClipKind::RichText, html: Some(html.clone()), file_id: None, file_name: None, mime_type: None, file_size: 0, file_expires_at_ms: 0, file_downloaded: false, local_file_path: None },
            IncomingPayload::Image { .. } => ClipItem { id: uuid::Uuid::new_v4().to_string(), content: "Image clipboard content".into(), source_device: resolved.source_device_name.clone(), source_device_id: resolved.source_device_id.clone(), received_at_ms: now_ms(), kind: ClipKind::Image, html: None, file_id: None, file_name: None, mime_type: Some("image/raw".into()), file_size: 0, file_expires_at_ms: 0, file_downloaded: false, local_file_path: None },
            IncomingPayload::FileOffer { file } => ClipItem { id: uuid::Uuid::new_v4().to_string(), content: file.name.clone(), source_device: resolved.source_device_name.clone(), source_device_id: resolved.source_device_id.clone(), received_at_ms: now_ms(), kind: ClipKind::File, html: None, file_id: Some(file.file_id.clone()), file_name: Some(file.name.clone()), mime_type: Some(file.mime_type.clone()), file_size: file.size, file_expires_at_ms: file.expires_at_ms, file_downloaded: false, local_file_path: None },
        };
        let entry_id = item.id.clone();
        if self.store.add(item) { self.notify_clipboard_changed(); }
        if resolved.decision != IncomingClipboardDecision::Copy { return; }
        match resolved.payload {
            IncomingPayload::Text(content) => { self.remember_hash(payload_hash(&ClipboardPayload::Text(content.clone()))); self.clipboard_sink.write_text(content); }
            IncomingPayload::RichText { text, html } => { self.remember_hash(payload_hash(&ClipboardPayload::RichText { text: text.clone(), html: html.clone() })); self.clipboard_sink.write_rich_text(text, html); }
            IncomingPayload::Image { width, height, rgba } => {
                self.remember_hash(payload_hash(&ClipboardPayload::Image { width, height, rgba: rgba.clone() }));
                match encode_png_rgba(width, height, &rgba).and_then(|png| self.clipboard_sink.save_file("clipx-image.png", "image/png", &png)) {
                    Ok(path) => {
                        if let Some(stored) = self.store.history.iter_mut().find(|i| i.id == entry_id) {
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
                        tokio::spawn(async move { notification.notify_info("Image saved", path).await; });
                    }
                    Err(e) => {
                        let notification = self.notification.clone();
                        tokio::spawn(async move { notification.notify_info("Image save failed", e).await; });
                    }
                }
            }
            IncomingPayload::FileOffer { .. } => {
                let _ = self.start_download(&entry_id);
            }
        }
    }

    fn finish_file_download(&mut self, source_device_id: String, file_id: String, path: PathBuf) {
        let Some(item) = self.store.history.iter().find(|i| i.file_id.as_deref() == Some(file_id.as_str())).cloned() else { return; };
        let entry_id = item.id.clone();
        if item.source_device_id != source_device_id {
            self.notify_transfer(&entry_id, &file_id, 0, item.file_size, "failed", "File data came from an unexpected device");
            return;
        }
        if item.file_expires_at_ms < now_ms() { let _ = fs::remove_file(&path); self.notify_transfer(&entry_id, &file_id, 0, item.file_size, "expired", "File offer expired"); return; }
        let name = item.file_name.clone().unwrap_or_else(|| "clipx-file".into());
        let mime_type = item.mime_type.clone().unwrap_or_else(|| "application/octet-stream".into());
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.notify_transfer(&entry_id, &file_id, 0, size, "saving", "Saving file");
        let result = self.clipboard_sink.save_file_from_path(&name, &mime_type, &path);
        let _ = fs::remove_file(&path);
        match result {
            Ok(path) => {
                let mut updated = item;
                updated.file_downloaded = true;
                updated.local_file_path = Some(path.clone());
                updated.content = name;
                updated.source_device_id = source_device_id;
                let _ = self.store.update(updated);
                self.notify_clipboard_changed();
                self.notify_transfer(&entry_id, &file_id, size, size, "complete", path);
            }
            Err(e) => self.notify_transfer(&entry_id, &file_id, 0, size, "failed", e),
        }
    }

    fn start_download(&mut self, entry_id: &str) -> String {
        let Some(item) = self.store.history.iter().find(|i| i.id == entry_id).cloned() else { return "Clipboard history item not found".into(); };
        if item.kind != ClipKind::File { return "Only file items can be downloaded".into(); }
        if item.file_expires_at_ms < now_ms() { return "File offer expired".into(); }
        let Some(file_id) = item.file_id else { return "File metadata is missing".into(); };
        let _ = self.outbound_tx.send(ClipboardOutbound::FileRequest { device_id: item.source_device_id, file_id: file_id.clone(), entry_id: entry_id.to_string() });
        self.notify_transfer(entry_id, &file_id, 0, item.file_size, "requesting", "Requesting file");
        "download requested".into()
    }

    fn cleanup_expired_files(&self) {
        let Ok(entries) = fs::read_dir(&self.files_dir) else { return; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") { continue; }
            let Ok(bytes) = fs::read(&path) else { continue; };
            let Ok(manifest) = serde_json::from_slice::<FileOfferManifest>(&bytes) else { continue; };
            if manifest.expires_at_ms >= now_ms() { continue; }
            let file_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let _ = fs::remove_file(self.files_dir.join(file_id));
            let _ = fs::remove_file(path);
        }
    }

    fn send_file_for_peer(&mut self, device_id: String, file_id: String) {
        let manifest_path = self.files_dir.join(format!("{file_id}.meta"));
        let manifest = match fs::read(&manifest_path).ok().and_then(|b| serde_json::from_slice::<FileOfferManifest>(&b).ok()) {
            Some(v) => v,
            None => { self.notify_info("File unavailable", "The file offer is no longer available"); return; }
        };
        if manifest.expires_at_ms < now_ms() {
            let _ = fs::remove_file(self.files_dir.join(&file_id));
            let _ = fs::remove_file(&manifest_path);
            self.notify_info("File expired", "The file offer has expired");
            return;
        }
        let path = self.files_dir.join(&file_id);
        let Ok(meta) = fs::metadata(&path) else { self.notify_info("File unavailable", "The offered file is missing"); return; };
        if meta.len() != manifest.size { self.notify_info("File changed", "The offered file is no longer the same size"); return; }
        let _ = self.outbound_tx.send(ClipboardOutbound::FileStream { device_id, file_id, path, total_size: manifest.size });
    }


}

fn payload_hash(p: &ClipboardPayload) -> u64 {
    let mut h = DefaultHasher::new();
    match p {
        ClipboardPayload::Text(s) => { 0u8.hash(&mut h); s.hash(&mut h); }
        ClipboardPayload::RichText { text, html } => { 1u8.hash(&mut h); text.hash(&mut h); html.hash(&mut h); }
        ClipboardPayload::Image { width, height, rgba } => { 2u8.hash(&mut h); width.hash(&mut h); height.hash(&mut h); rgba.hash(&mut h); }
        ClipboardPayload::Files(paths) => { 3u8.hash(&mut h); paths.hash(&mut h); }
        ClipboardPayload::FileBytes { name, mime_type, data } => { 4u8.hash(&mut h); name.hash(&mut h); mime_type.hash(&mut h); data.hash(&mut h); }
        ClipboardPayload::FilePath { path, name, mime_type } => { 5u8.hash(&mut h); path.hash(&mut h); name.hash(&mut h); mime_type.hash(&mut h); }
    }
    h.finish()
}

fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize).checked_mul(height as usize).and_then(|v| v.checked_mul(4)).ok_or("image dimensions overflow")?;
    if width == 0 || height == 0 || rgba.len() != expected { return Err("invalid RGBA image data".into()); }
    fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = 0xffff_ffffu32;
        for b in kind.iter().chain(data.iter()) {
            crc ^= *b as u32;
            for _ in 0..8 { crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 }; }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0);
        let start = row * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    let mut pos = 0usize;
    let mut a = 1u32;
    let mut b = 0u32;
    while pos < raw.len() {
        let n = (raw.len() - pos).min(65_535);
        let final_block = pos + n == raw.len();
        z.push(if final_block { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        for &byte in &raw[pos..pos+n] { a = (a + byte as u32) % 65_521; b = (b + a) % 65_521; }
        z.extend_from_slice(&raw[pos..pos+n]);
        pos += n;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr, &mut out);
    chunk(b"IDAT", &z, &mut out);
    chunk(b"IEND", &[], &mut out);
    Ok(out)
}

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }
fn mime_from_name(name: &str) -> String {
    match std::path::Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp", "pdf" => "application/pdf", "txt" => "text/plain", "html" | "htm" => "text/html", _ => "application/octet-stream",
    }.to_string()
}

fn format_size(size: u64) -> String { if size >= 1 << 30 { format!("{:.1} GB", size as f64 / (1 << 30) as f64) } else if size >= 1 << 20 { format!("{:.1} MB", size as f64 / (1 << 20) as f64) } else if size >= 1 << 10 { format!("{:.0} KB", size as f64 / (1 << 10) as f64) } else { format!("{size} B") } }
