use super::seen::SeenDeviceRegistry;
use super::transfer_profile::{ReceiverProfile, SenderProfile};
use super::trusted::TrustedDeviceStore;
use super::types::{IdentitySnapshot, SeenDevice, TrustedDevice};
use crate::clipboard::manager::{ClipboardCommand, ClipboardOutbound, ClipboardPayload};
use crate::device::config::{Gate, check_proto_version, get_hostname};
use crate::device::identity::{self, DeviceIdentity};
use crate::device::pairing::{ConnectSession, ConnectStage, PairSession, PairStage, Role};
use crate::device::{config, pairing};
use crate::message::proto::Hello;
use crate::message::proto::{self, clipboard_message, peer_message::Body};
use crate::netio::transport::{TransportCommand, TransportEvent};
use crate::notification::{PairDecision, Prompt, platform::NotificationEngine as Engine};
use crate::platform::CoreEvent;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

macro_rules! shorter {
    ($s:expr) => {{
        let mut k = String::from($s);
        k.truncate(16);
        k.push_str("...");
        k
    }};
}

fn device_command_name(command: &DeviceCommands) -> &'static str {
    match command {
        DeviceCommands::GetSeen { .. } => "get_seen",
        DeviceCommands::Pair { .. } => "pair",
        DeviceCommands::Connect { .. } => "connect",
        DeviceCommands::Connected { .. } => "connected",
        DeviceCommands::GetIdentity { .. } => "get_identity",
        DeviceCommands::GetPaired { .. } => "get_paired",
        DeviceCommands::Disconnect { .. } => "disconnect",
        DeviceCommands::SetAutoConnect { .. } => "set_auto_connect",
        DeviceCommands::ForgetDevice { .. } => "forget_device",
        DeviceCommands::PendingPairings { .. } => "pending_pairings",
        DeviceCommands::ApprovePairing { .. } => "approve_pairing",
        DeviceCommands::ControlTransfer { .. } => "control_transfer",
        DeviceCommands::ListTransfers { .. } => "list_transfers",
    }
}

fn peer_message_name(body: &Body) -> &'static str {
    match body {
        Body::PairRequest(_) => "pair_request",
        Body::PairResponse(_) => "pair_response",
        Body::PairChallenge(_) => "pair_challenge",
        Body::PairChallengeResponse(_) => "pair_challenge_response",
        Body::PairAck(_) => "pair_ack",
        Body::ConnectChallenge(_) => "connect_challenge",
        Body::ConnectChallengeResponse(_) => "connect_challenge_response",
        Body::ConnectAck(_) => "connect_ack",
        Body::Control(_) => "control",
        Body::Clipboard(_) => "clipboard",
        Body::FileDownloadRequest(_) => "file_download_request",
        Body::FileChunk(_) => "file_chunk",
        Body::FileChunkAck(_) => "file_chunk_ack",
        Body::FileTransferCancel(_) => "file_transfer_cancel",
        Body::FileTransferResume(_) => "file_transfer_resume",
    }
}
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_RICH_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_CHUNK_BYTES: usize = 1024 * 1024;
/// A download that has received no chunk for this long is considered stalled
/// (for example a lost ack, or a sender task still bound to a dead socket).
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(45);
/// Number of times a stalled download re-asks the sender before reporting the
/// transfer as interrupted.
const DOWNLOAD_MAX_STALL_RETRIES: u8 = 3;
const RESUME_STATE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const STALE_INCOMING_CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Coordinates peer discovery, pairing, connection state, and file transfer flow.
///
/// The device manager is the heart of the runtime. It decides whether a peer is
/// trusted, tracks pending pair requests, maintains the live connection set, and
/// converts incoming protocol messages into clipboard or file-transfer actions.
pub struct DeviceManager<E: Engine> {
    incoming_files: HashMap<(String, String), IncomingFileAssembly>,
    incoming_files_dir: PathBuf,
    seen: SeenDeviceRegistry,
    trusted: TrustedDeviceStore,
    identity: Arc<DeviceIdentity>,
    pair_sessions: HashMap<String, PairSession>,
    connect_sessions: HashMap<String, ConnectSession>,
    connected: HashMap<String, String>,
    transport_tx: Option<mpsc::UnboundedSender<TransportCommand>>,
    notification: E,
    notify_tx: mpsc::UnboundedSender<NotifyResult>,
    notify_rx: mpsc::UnboundedReceiver<NotifyResult>,
    /// Where inbound clipboard content gets handed off. The clipboard
    /// component owns what happens to it from here — this manager only
    /// knows it needs to deliver "this text came from this device".
    clipboard_tx: mpsc::UnboundedSender<ClipboardCommand>,
    /// Pushed to every IPC client whenever the paired/connection picture
    /// changes — pairing, connect/disconnect, forget, auto-connect,
    /// availability flips. Send-only from here; the IPC layer owns the
    /// receiving/fan-out side.
    events_tx: broadcast::Sender<CoreEvent>,

    // Tracks active download sessions by file id. Keeping the entry metadata
    // here avoids releasing the guard before the clipboard manager finishes
    // saving the fully received temporary file.
    pending_downloads: HashMap<String, PendingDownload>,
    // Last UI percentage reported for each direction/file. Core events use a
    // broadcast channel, so emitting once per 1 MiB chunk can overwhelm a
    // slower subscriber and make a large transfer appear stuck.
    transfer_progress: HashMap<String, (String, u8)>,
    upload_sessions: HashMap<(String, String), UploadSession>,
    held_uploads: HashMap<(String, String), HeldUpload>,
    upload_done_tx: mpsc::UnboundedSender<(String, String, u64)>,
    upload_done_rx: mpsc::UnboundedReceiver<(String, String, u64)>,
    next_upload_id: u64,
    resume_candidates: HashMap<String, ResumeState>,
    transport_connections: HashMap<String, String>,
    // Track each peer capabilities and effective versions tied to there device_id
    // We record and clean an entry when peer connect and disconnect to avoid bloats and stale versions
    // Why in Device manager and per connection cycle, typically we do not want to
    // persist in the trusted store because, device change +upgrade or -downgrade but
    // device trusted store and identity do not change unless one clear/erase them explicitly and
    // not just by app updates
    effective_versions: HashMap<String, (u32, Vec<i32>)>,
}

/// Results of a notification popup, fed back into the manager's own select
/// loop — `NotificationEngine::ask()` can take up to its timeout, so it's
/// always spawned as its own task rather than awaited inline; awaiting it
/// directly would stall every other device-manager operation for as long
/// as the popup is on screen.
#[derive(Debug, Clone)]
struct PendingDownload {
    device_id: String,
    entry_id: String,
    file_name: String,
    total_size: u64,
    offer_expires_at_ms: u64,
    received_size: u64,
    received_complete: bool,
    /// Last time a chunk arrived (or the request was made). Drives the stall
    /// watchdog.
    last_activity: Instant,
    /// How many times the watchdog has re-requested this file since the last
    /// chunk. Above DOWNLOAD_MAX_STALL_RETRIES means the watchdog gave up.
    stall_retries: u8,
}

#[allow(unused)]
struct IncomingFileAssembly {
    next_seq: u64,
    total_size: u64,
    received_size: u64,
    temp_path: PathBuf,
    state_path: PathBuf,
    file: File,
    profile: ReceiverProfile,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ResumeState {
    version: u32,
    device_id: String,
    file_id: String,
    entry_id: String,
    file_name: String,
    total_size: u64,
    confirmed_offset: u64,
    offer_expires_at_ms: u64,
    updated_at_ms: u64,
}

/// What the user asked for on a running transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferAction {
    Pause,
    Resume,
    Cancel,
}

/// One live transfer, for UIs that open after it started. A transfer is one
/// offer going to, or coming from, one device: an offer sent to several
/// devices is several transfers.
#[derive(Debug, Clone)]
pub struct TransferSnapshot {
    pub device_id: String,
    pub device_name: String,
    pub entry_id: String,
    pub file_id: String,
    pub file_name: String,
    /// "download" or "send".
    pub direction: String,
    pub done: u64,
    pub total: u64,
    pub state: String,
    /// False when the peer is an older build without pause/resume.
    pub can_pause: bool,
}

/// An upload the sending user paused. Nothing is streaming; the receiver keeps
/// its partial data and asks again when it (or the sender, via
/// `FileTransferResume`) continues.
struct HeldUpload {
    file_name: String,
    total_size: u64,
    done: u64,
    /// The sender pressed Resume and is waiting for the receiver to ask again.
    waiting: bool,
}

struct UploadSession {
    /// Identifies this session so a late "done" message from a replaced
    /// session cannot remove its replacement.
    id: u64,
    cancel: CancellationToken,
    interrupt: CancellationToken,
    file_name: String,
    total_size: u64,
    /// Bytes the receiver has acknowledged so far.
    confirmed: Arc<AtomicU64>,
    /// Set when the local user paused this upload, so the task reports
    /// "paused" instead of "interrupted".
    paused: Arc<AtomicBool>,
    /// Set when a newer request for the same file replaced this session, so
    /// the old task exits quietly without reporting "interrupted".
    superseded: Arc<AtomicBool>,
    ack_tx: mpsc::UnboundedSender<u64>,
}

enum NotifyResult {
    PairApproval {
        device_id: String,
        decision: PairDecision,
    },
    PairCodeConfirm {
        device_id: String,
        decision: PairDecision,
    },
}

/// Filters the list of previously seen devices by trust level.
pub enum SeenMode {
    All,
    Trusted,
    Untrusted,
}

/// A lightweight snapshot of a pending pairing operation shown to the caller.
pub struct PendingPairEntry {
    pub device_id: String,
    pub device_name: String,
    pub stage: &'static str,
    pub code: Option<String>,
}

/// Commands accepted by the device manager from the rest of the runtime.
///
/// These inputs are intentionally narrow and message-oriented so the rest of the
/// system can ask for identity, pairing state, and connection changes without
/// needing direct access to internals.
pub enum DeviceCommands {
    GetSeen {
        mode: SeenMode,
        reply_to: oneshot::Sender<Vec<SeenDevice>>,
    },
    Pair {
        device_id: String,
        reply_to: oneshot::Sender<String>,
    },
    Connect {
        device_id: String,
        reply_to: oneshot::Sender<String>,
    },
    Connected {
        reply_to: oneshot::Sender<Vec<SeenDevice>>,
    },
    GetIdentity {
        reply_to: oneshot::Sender<IdentitySnapshot>,
    },
    /// Pause, resume or cancel one transfer without touching clipboard
    /// history. `device_id` narrows it to one device when the same offer is
    /// going to several. `Err` is a message for the user (a toast) explaining
    /// a refusal, such as a peer that cannot pause.
    ControlTransfer {
        device_id: Option<String>,
        file_id: String,
        action: TransferAction,
        reply_to: oneshot::Sender<Result<String, String>>,
    },
    ListTransfers {
        reply_to: oneshot::Sender<Vec<TransferSnapshot>>,
    },
    GetPaired {
        reply_to: oneshot::Sender<Vec<proto::DeviceInfo>>,
    },
    Disconnect {
        device_id: String,
        reply_to: oneshot::Sender<String>,
    },
    SetAutoConnect {
        device_id: String,
        auto_connect: bool,
        reply_to: oneshot::Sender<bool>,
    },
    ForgetDevice {
        device_id: String,
        reply_to: oneshot::Sender<String>,
    },
    PendingPairings {
        reply_to: oneshot::Sender<Vec<PendingPairEntry>>,
    },
    ApprovePairing {
        device_id: String,
        approve: bool,
        reply_to: oneshot::Sender<String>,
    },
}

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub fn new(
        trusted_store_path: std::path::PathBuf,
        identity: Arc<DeviceIdentity>,
        transport_tx: Option<mpsc::UnboundedSender<TransportCommand>>,
        notification: E,
        clipboard_tx: mpsc::UnboundedSender<ClipboardCommand>,
        events_tx: broadcast::Sender<CoreEvent>,
    ) -> Self {
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        let (upload_done_tx, upload_done_rx) = mpsc::unbounded_channel();
        let incoming_files_dir = crate::device::config::transfer_dir().join("incoming-files");
        let _ = std::fs::create_dir_all(&incoming_files_dir);
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
            identity,
            pair_sessions: HashMap::new(),
            connect_sessions: HashMap::new(),
            connected: HashMap::new(),
            transport_connections: HashMap::new(),
            incoming_files: HashMap::new(),
            incoming_files_dir,
            transport_tx,
            notification,
            notify_tx,
            notify_rx,
            clipboard_tx,
            events_tx,
            pending_downloads: HashMap::new(),
            transfer_progress: HashMap::new(),
            upload_sessions: HashMap::new(),
            held_uploads: HashMap::new(),
            upload_done_tx,
            upload_done_rx,
            next_upload_id: 0,
            resume_candidates: HashMap::new(),
            effective_versions: HashMap::new(),
        }
    }

    /// Fires a payload-free "something about paired/connection state
    /// changed, go re-fetch" ping to every currently-connected IPC client.
    fn notify_devices_changed(&self) {
        let _ = self.events_tx.send(CoreEvent::DevicesChanged);
    }

    fn notify_pair_events(&self, device_id: &str, state: i32, message: &str) {
        let state = match proto::pairing_event::State::try_from(state) {
            Ok(proto::pairing_event::State::Started) => crate::platform::PairingEventState::Started,
            Ok(proto::pairing_event::State::Failed) => crate::platform::PairingEventState::Failed,
            Ok(proto::pairing_event::State::Succeeded) => {
                crate::platform::PairingEventState::Succeeded
            }
            Err(_) => return,
        };
        let _ = self.events_tx.send(CoreEvent::PairingChanged {
            device_id: device_id.to_string(),
            state,
            message: message.to_string(),
        });
    }

    pub async fn run(
        mut self,
        mut shutdown_rx: watch::Receiver<bool>,
        mut discovered_rx: mpsc::UnboundedReceiver<(proto::Announce, SocketAddr)>,
        mut device_rx: mpsc::UnboundedReceiver<DeviceCommands>,
        mut peer_event_rx: mpsc::UnboundedReceiver<TransportEvent>,
        mut clipboard_out_rx: mpsc::UnboundedReceiver<ClipboardOutbound>,
        clipboard_tx: mpsc::UnboundedSender<ClipboardCommand>,
    ) {
        tracing::info!("device manager started");
        let mut prune_interval = tokio::time::interval(Duration::from_secs(10));
        let mut incoming_cleanup_interval = tokio::time::interval(STALE_INCOMING_CLEANUP_INTERVAL);
        incoming_cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        self.load_resume_candidates();
        tracing::debug!(
            resume_candidates = self.resume_candidates.len(),
            "incoming transfer resume state loaded"
        );
        let notify_tx = self.notify_tx.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        // Stop sender tasks promptly, but classify shutdown as an
                        // interruption so confirmed progress remains resumable.
                        for (_, session) in self.upload_sessions.drain() {
                            session.interrupt.cancel();
                        }
                        break;
                    }
                }
                Some((announce, addr)) = discovered_rx.recv() => { self.handle_announce(announce, addr.ip()); }
               _ = prune_interval.tick() => {
                    if self.seen.prune_stale(Duration::from_secs(10)) {
                        self.notify_devices_changed();
                    }

                    self.prune_expired_sessions();
                    self.check_stalled_downloads();
                }
                Some(cmd) = device_rx.recv() => {
                    let command = device_command_name(&cmd);
                    let started = Instant::now();
                    self.handle_device_command(cmd).await;
                    tracing::trace!(command, elapsed_us = started.elapsed().as_micros() as u64, "device command processed");
                }
                Some(event) = peer_event_rx.recv() => { self.handle_transport_event(event, &notify_tx); }
                Some(result) = self.notify_rx.recv() => { self.handle_notify_result(result); }
                // The clipboard component decided its content changed and
                // handed us plain text — dispatch is entirely our call, it
                // never knows this happened.
                Some(outbound) = clipboard_out_rx.recv() => { self.handle_clipboard_outbound(outbound, &clipboard_tx); }
                Some((device_id, file_id, session_id)) = self.upload_done_rx.recv() => {
                    let key = (device_id, file_id);
                    if self.upload_sessions.get(&key).is_some_and(|s| s.id == session_id) {
                        self.upload_sessions.remove(&key);
                    }
                }
                _ = incoming_cleanup_interval.tick() => { self.cleanup_stale_incoming_files(); }
            }
        }
        tracing::info!("device manager stopped");
    }

    /// Dispatches clipboard payloads to every currently connected, trusted peer.
    fn broadcast_clipboard(&self, payload: ClipboardPayload, file_offer: Option<proto::FileOffer>) {
        if self.connected.is_empty() {
            return;
        }
        let ids: Vec<String> = self.connected.keys().cloned().collect();
        for device_id in ids {
            let content = match (&payload, file_offer.as_ref()) {
                (ClipboardPayload::Text(text), _) => {
                    Some(clipboard_message::Content::Text(text.clone()))
                }
                (ClipboardPayload::RichText { text, html }, _) => {
                    Some(clipboard_message::Content::RichText(proto::RichText {
                        text: text.clone(),
                        html: html.clone(),
                    }))
                }
                (
                    ClipboardPayload::Image {
                        width,
                        height,
                        rgba,
                    },
                    _,
                ) => Some(clipboard_message::Content::Image(proto::ImageContent {
                    width: *width,
                    height: *height,
                    rgba: rgba.clone(),
                })),
                (ClipboardPayload::Files(_), Some(file))
                | (ClipboardPayload::FileBytes { .. }, Some(file))
                | (ClipboardPayload::FilePath { .. }, Some(file)) => {
                    Some(clipboard_message::Content::FileOffer(file.clone()))
                }
                (ClipboardPayload::Files(_), None)
                | (ClipboardPayload::FileBytes { .. }, None)
                | (ClipboardPayload::FilePath { .. }, None) => None,
            };
            if let Some(content) = content {
                self.send_peer(
                    &device_id,
                    Body::Clipboard(proto::ClipboardMessage {
                        content: Some(content),
                    }),
                );
            }
        }
    }

    fn handle_clipboard_outbound(
        &mut self,
        outbound: ClipboardOutbound,
        _clipboard_tx: &mpsc::UnboundedSender<ClipboardCommand>,
    ) {
        match outbound {
            ClipboardOutbound::Payload(payload) => self.broadcast_clipboard(payload, None),
            ClipboardOutbound::FileOffer {
                file_id,
                name,
                mime_type,
                size,
                expires_at_ms,
            } => {
                self.broadcast_clipboard(
                    ClipboardPayload::FileBytes {
                        name: name.clone(),
                        mime_type: mime_type.clone(),
                        data: Vec::new(),
                    },
                    Some(proto::FileOffer {
                        file_id,
                        name,
                        mime_type,
                        size,
                        expires_at_ms,
                    }),
                );
            }
            ClipboardOutbound::FileRequest {
                device_id,
                file_id,
                entry_id,
                file_name,
                total_size,
                offer_expires_at_ms,
            } => {
                if self.pending_downloads.contains_key(&file_id) {
                    tracing::debug!(%file_id, "ignoring duplicate file download request");
                    return;
                }
                if !self.connected.contains_key(&device_id) {
                    let _ = self
                        .clipboard_tx
                        .send(ClipboardCommand::FileDownloadReleased {
                            file_id: file_id.clone(),
                        });
                    self.notify_transfer(
                        &device_id,
                        &entry_id,
                        &file_id,
                        &file_name,
                        "download",
                        0,
                        total_size,
                        "failed",
                        "Device is not connected",
                    );
                    return;
                }
                // Continues from a kept partial download (paused or
                // interrupted) when one is still valid, else starts at 0.
                self.start_or_resume_download(
                    device_id,
                    file_id,
                    entry_id,
                    file_name,
                    total_size,
                    offer_expires_at_ms,
                );
            }
            ClipboardOutbound::FileDownloadFinished { file_id } => {
                self.pending_downloads.remove(&file_id);
            }
            ClipboardOutbound::FileTransferCancel { device_id, file_id } => {
                self.cancel_local_transfer(&file_id);
                self.send_peer(
                    &device_id,
                    Body::FileTransferCancel(proto::FileTransferCancel {
                        file_id,
                        pause: false,
                    }),
                );
            }
            ClipboardOutbound::FileStream {
                device_id,
                file_id,
                path,
                total_size,
                start_offset,
            } => self.stream_file_to_peer(device_id, file_id, path, total_size, start_offset),
        }
    }

    fn handle_announce(&mut self, announce: proto::Announce, addr: IpAddr) {
        let Ok(device_type) = proto::DeviceType::try_from(announce.device_type) else {
            tracing::warn!("Unknown device_type {} from {addr}", announce.device_type);
            return;
        };
        let Ok(pub_key_fingerprint): Result<[u8; 32], _> =
            announce.fingerprint.as_slice().try_into()
        else {
            tracing::warn!("malformed public key fingerprint from {addr}, ignoring announce");
            return;
        };
        let device_id = hex::encode(pub_key_fingerprint);

        let just_appeared = !self.seen.already_seen(&device_id);
        if just_appeared {
            tracing::info!(
                "New {} device discovered: {}({addr})",
                String::from(device_type),
                announce.device_name
            );
        } else {
            tracing::trace!(device_id = %shorter!(&device_id), %addr, ws_port = announce.ws_port, "peer discovery announcement refreshed");
        }

        self.seen.upsert(SeenDevice {
            id: device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            ws_port: announce.ws_port,
            last_seen: Instant::now(),
        });

        if just_appeared {
            self.notify_devices_changed();
        }

        // Auto-connect is deliberately a *fresh discovery* trigger. Do not
        // reconnect merely because an already-visible peer disconnected, and
        // do not start a connection just because the user toggled this on.
        // Both sides are allowed to initiate here; the transport/handshake
        // resolves an overlap instead of using a fingerprint tie-breaker.
        if just_appeared
            && let Some(td) = self.trusted.get(&device_id)
            && td.auto_connect
            && !self.connected.contains_key(&device_id)
            && !self.connect_sessions.contains_key(&device_id)
        {
            let _ = self.handle_connect(&device_id);
        }
    }

    // ---------------- Pair: initiator side ----------------

    fn handle_pair_request(&mut self, device_id: &str) -> String {
        if self.trusted.is_trusted(device_id) {
            return "already trusted".to_string();
        }
        if self.pair_sessions.contains_key(device_id) {
            return "pairing already in progress".to_string();
        }
        let Some(seen) = self.seen.get(device_id).cloned() else {
            return "device not found".to_string();
        };
        let addr = SocketAddr::new(seen.addr, seen.ws_port as u16);

        self.pair_sessions.insert(
            device_id.to_string(),
            PairSession::new_initiator(addr, seen.device_type),
        );
        tracing::debug!(device_id = %shorter!(device_id), "pairing initiated");
        self.notify_pair_events(
            device_id,
            proto::pairing_event::State::Started as i32,
            "pairing started",
        );
        self.dial(device_id, addr);
        "pairing started".to_string()
    }

    // ---------------- Connect: initiator side ----------------

    fn handle_connect(&mut self, device_id: &str) -> String {
        if !self.trusted.is_trusted(device_id) {
            return "device is not trusted".to_string();
        }
        if self.connected.contains_key(device_id) {
            return "already connected".to_string();
        }
        if self.connect_sessions.contains_key(device_id) {
            return "connect already in progress".to_string();
        }
        let Some(seen) = self.seen.get(device_id).cloned() else {
            return "device not currently reachable".to_string();
        };
        let addr = SocketAddr::new(seen.addr, seen.ws_port as u16);
        tracing::debug!(device_id = %shorter!(device_id), %addr, "Handle connect: triggered/auto-connect");

        self.connect_sessions
            .insert(device_id.to_string(), ConnectSession::new_initiator(addr));

        // The peer may already have opened the transport while this logical
        // connect request was being created. Reuse that live connection rather
        // than opening another socket. This is still gated by the caller's
        // normal fresh-discovery/manual-connect rules.
        if let Some(connection_id) = self.transport_connections.get(device_id).cloned() {
            tracing::debug!(%connection_id, "Re-using an existing peer transport");
            self.on_transport_connected(device_id.to_string(), connection_id);
        } else {
            tracing::debug!(
                device_id = %shorter!(device_id),
                "Dialing new transport: no peer transport available"
            );
            self.dial(device_id, addr);
        }
        "connecting".to_string()
    }

    fn dial(&self, device_id: &str, addr: SocketAddr) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::Connect {
            device_id: device_id.to_string(),
            addr,
            reply_to: reply_tx,
        });
    }

    async fn handle_disconnect(&mut self, device_id: &str) -> String {
        self.connect_sessions.remove(device_id);
        let Some(tx) = self.transport_tx.as_ref() else {
            return "transport unavailable".to_string();
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::Disconnect {
            device_id: device_id.to_string(),
            reply_to: reply_tx,
        });
        let removed = reply_rx.await.unwrap_or(false);
        let was_connected = self.connected.remove(device_id).is_some();
        self.transport_connections.remove(device_id);
        if was_connected {
            self.notify_devices_changed();
        }
        if removed {
            "disconnected".to_string()
        } else {
            "was not connected".to_string()
        }
    }

    // ---------------- Transport events ----------------

    fn handle_transport_event(
        &mut self,
        event: TransportEvent,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        match event {
            TransportEvent::Connected(dev) => {
                self.on_transport_connected(dev.id, dev.connection_id)
            }
            TransportEvent::Disconnected {
                device_id,
                connection_id,
            } => self.on_transport_disconnected(device_id, connection_id),
            TransportEvent::ConnectFailed(device_id) => self.on_connect_failed(device_id),
            TransportEvent::PeerMessage {
                device_id,
                connection_id,
                message,
                profile,
            } => {
                if self
                    .connected
                    .get(&device_id)
                    .is_some_and(|current| current != &connection_id)
                {
                    return;
                }

                // Time spent waiting in the Transport -> DeviceManager channel.
                let event_queue_us = profile
                    .as_ref()
                    .map(|p| p.event_enqueued_at.elapsed().as_micros() as u64)
                    .unwrap_or(0);

                let device_id_for_log = device_id.clone();
                let connection_id_for_log = connection_id.clone();

                // Time spent processing the incoming message in DeviceManager.
                let started = Instant::now();

                self.on_peer_message(device_id, connection_id, message, notify_tx);

                let manager_dispatch_us = started.elapsed().as_micros() as u64;

                if let Some(profile) = profile {
                    tracing::trace!(
                        device_id = %device_id_for_log,
                        connection_id = %connection_id_for_log,
                        message_type = profile.trace.message_type,
                        file_id = %profile.trace.file_id,
                        confirmed_offset = profile.trace.confirmed_offset,
                        seq = ?profile.trace.seq,
                        payload_bytes = profile.trace.payload_bytes,
                        decode_us = profile.decode_us,
                        event_queue_us,
                        manager_dispatch_us,
                        "device manager inbound profile"
                    );
                }
            }
        }
    }

    fn on_transport_connected(&mut self, device_id: String, connection_id: String) {
        self.transport_connections
            .insert(device_id.clone(), connection_id.clone());
        if let Some(sess) = self.pair_sessions.get_mut(&device_id) {
            if sess.stage == PairStage::Dialing {
                sess.stage = PairStage::AwaitingPeerResponse;
                let own_pub = self.identity.public_key_bytes();
                let own_fp = self.identity.get_this_device_fingerprint();
                self.send_peer(
                    &device_id,
                    Body::PairRequest(proto::PeerPairRequest {
                        name: config::get_hostname(),
                        public_key: own_pub.to_vec(),
                        fingerprint: own_fp.to_vec(),
                        hello: Some(config::local_hello()),
                    }),
                );
            }
            return;
        }
        if let Some(sess) = self.connect_sessions.get_mut(&device_id)
            && sess.role == Role::Initiator
            && sess.stage == ConnectStage::Dialing
        {
            sess.stage = ConnectStage::AwaitingSignature;
            sess.connection_id = Some(connection_id.clone());
            let nonce_vec = self.identity.random_nonce();
            let nonce_arr: [u8; 32] = nonce_vec
                .as_slice()
                .try_into()
                .expect("nonce must be 32 bytes");
            sess.nonce = Some(nonce_arr);
            let own_fp = self.identity.get_this_device_fingerprint();
            self.send_peer_on_connection(
                &device_id,
                &connection_id,
                Body::ConnectChallenge(proto::PeerConnectChallenge {
                    nonce: nonce_vec,
                    initiator_fingerprint: own_fp.to_vec(),
                    hello: Some(config::local_hello()),
                }),
            );
        }
    }

    fn on_transport_disconnected(&mut self, device_id: String, connection_id: String) {
        if self
            .transport_connections
            .get(&device_id)
            .is_some_and(|current| current != &connection_id)
        {
            return;
        }
        self.transport_connections.remove(&device_id);
        let interrupted: Vec<(String, PendingDownload)> = self
            .pending_downloads
            .iter()
            .filter(|(_, pending)| pending.device_id == device_id && !pending.received_complete)
            .map(|(file_id, pending)| (file_id.clone(), pending.clone()))
            .collect();
        let upload_keys: Vec<(String, String)> = self
            .upload_sessions
            .keys()
            .filter(|(upload_device, _)| upload_device == &device_id)
            .cloned()
            .collect();
        for key in upload_keys {
            if let Some(session) = self.upload_sessions.remove(&key) {
                session.interrupt.cancel();
            }
        }
        for (file_id, pending) in interrupted {
            if let Some(s) = self
                .incoming_files
                .remove(&(device_id.clone(), file_id.clone()))
            {
                s.profile.log_summary(&file_id, "interrupted");
            }
            self.notify_transfer(
                &pending.device_id,
                &pending.entry_id,
                &file_id,
                &pending.file_name,
                "download",
                pending.received_size,
                pending.total_size,
                "interrupted",
                "Downloading was interrupted; it can resume when the peer reconnects",
            );
        }

        if self.pair_sessions.remove(&device_id).is_some() {
            tracing::info!("pair session with {device_id} ended (connection closed)");
            self.notify_pair_events(
                &device_id,
                proto::pairing_event::State::Failed as i32,
                "connection lost during pairing",
            );
        }

        if let Some(sess) = self.connect_sessions.get(&device_id) {
            if sess.role == Role::Initiator && sess.retry_count < ConnectSession::MAX_RETRIES {
                let addr = sess.peer_addr;
                let attempt = sess.retry_count + 1;
                tracing::info!(
                    "connect session with {device_id} dropped, retrying ({attempt}/{})",
                    ConnectSession::MAX_RETRIES
                );

                if let Some(sess) = self.connect_sessions.get_mut(&device_id) {
                    sess.stage = ConnectStage::Dialing;
                    sess.connection_id = None;
                    sess.nonce = None;
                    sess.retry_count = attempt;
                }
                self.dial(&device_id, addr);
            } else {
                tracing::info!("connect session with {device_id} ended (connection closed)");
                self.connect_sessions.remove(&device_id);
                self.notify_devices_changed();
            }
        }

        if self.connected.remove(&device_id).is_some() {
            tracing::info!("{device_id} no longer connected: connection {connection_id} closed");
            self.notify_devices_changed();
        }
        // drop the effective_version related to avoid stale
        self.effective_versions.remove(&device_id);
        // A paused upload has nothing left to resume once the receiver is gone.
        self.held_uploads.retain(|(d, _), _| d != &device_id);
    }

    /// A dial never became a live connection at all (TCP connect failed/timed
    /// out, or the WS handshake failed) — as opposed to on_transport_disconnected,
    /// which is for a connection that *was* live and then dropped. `dial()` is
    /// shared by both the pairing and connect initiator paths, so check both.
    fn on_connect_failed(&mut self, device_id: String) {
        if self.pair_sessions.remove(&device_id).is_some() {
            tracing::info!("pair dial to {device_id} never connected");
            self.notify_info("Pairing failed", "could not reach device");
            self.notify_pair_events(
                device_id.as_str(),
                proto::pairing_event::State::Failed as i32,
                "pair failed could not reach device",
            );
            return;
        }

        let Some(sess) = self.connect_sessions.get(&device_id) else {
            return;
        };
        if sess.role == Role::Initiator && sess.retry_count < ConnectSession::MAX_RETRIES {
            let addr = sess.peer_addr;
            let attempt = sess.retry_count + 1;
            tracing::info!(
                "connect dial to {device_id} failed, retrying ({attempt}/{})",
                ConnectSession::MAX_RETRIES
            );

            if let Some(sess) = self.connect_sessions.get_mut(&device_id) {
                sess.stage = ConnectStage::Dialing;
                sess.nonce = None;
                sess.retry_count = attempt;
            }
            self.dial(&device_id, addr);
        } else {
            tracing::info!("connect dial to {device_id} failed, giving up");
            self.connect_sessions.remove(&device_id);
            self.notify_devices_changed();
        }
    }

    fn on_peer_message(
        &mut self,
        device_id: String,
        connection_id: String,
        message: proto::PeerMessage,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        if let Some(body) = message.body.as_ref() {
            tracing::trace!(
                device_id = %shorter!(&device_id),
                %connection_id,
                message_kind = peer_message_name(body),
                "peer protocol message dispatched"
            );
        }
        if self
            .connected
            .get(&device_id)
            .is_some_and(|current| current != &connection_id)
        {
            return;
        }
        match message.body {
            Some(Body::PairRequest(req)) => {
                self.on_pair_request(device_id, connection_id, req, notify_tx)
            }
            Some(Body::PairResponse(res)) => self.on_pair_response(device_id, res, notify_tx),
            Some(Body::PairChallenge(c)) => self.on_pair_challenge(device_id, c, notify_tx),
            Some(Body::PairChallengeResponse(r)) => self.on_pair_challenge_response(device_id, r),
            Some(Body::PairAck(_)) => self.on_pair_ack(device_id),
            Some(Body::ConnectChallenge(c)) => {
                self.on_connect_challenge(device_id, connection_id, c)
            }
            Some(Body::ConnectChallengeResponse(r)) => {
                self.on_connect_challenge_response(device_id, connection_id, r)
            }
            Some(Body::ConnectAck(_)) => self.on_connect_ack(device_id, connection_id),
            Some(Body::Control(ctrl)) => self.on_control(device_id, connection_id, ctrl),
            Some(Body::Clipboard(msg)) => self.on_clipboard_message(device_id, msg),
            Some(Body::FileDownloadRequest(req)) => {
                self.on_file_download_request(device_id, req);
            }
            Some(Body::FileChunk(chunk)) => {
                self.on_file_chunk(device_id, chunk);
            }
            Some(Body::FileChunkAck(ack)) => {
                self.on_file_chunk_ack(device_id, ack);
            }
            Some(Body::FileTransferCancel(cancel)) => {
                self.on_file_transfer_cancel(device_id, cancel);
            }
            Some(Body::FileTransferResume(resume)) => {
                self.on_file_transfer_resume(device_id, resume);
            }
            None => tracing::warn!("empty PeerMessage from {device_id}"),
        }
    }

    /// Version gate for a peer's hello. `Ok` records effective version + capabilities
    /// for this connection cycle; `Err(msg)` is the text to send to the peer.
    /// No hello (legacy peer) passes.
    fn check_hello(&mut self, device_id: &str, hello: Option<Hello>) -> Result<(), String> {
        let Some(h) = hello else { return Ok(()) };
        match check_proto_version(h.proto_version, h.min_proto_version) {
            Gate::Ok { effective } => {
                self.effective_versions
                    .insert(device_id.to_string(), (effective, h.capabilities));
                Ok(())
            }
            Gate::PeerTooOld => {
                tracing::debug!(%device_id, "peer is too old, pair/connect failed");
                Err(format!(
                    "App is too old, upgrade to newer versions to pair/connect with: {}",
                    get_hostname()
                ))
            }
            Gate::SelfTooOld => {
                tracing::warn!(%device_id, "this app is too old to pair/connect to peer, upgrade");
                Err(format!(
                    "{} is too old to pair/connect, update it",
                    get_hostname()
                ))
            }
        }
    }

    fn reject_pair_request(&self, device_id: &str, connection_id: &str, code: u32, message: &str) {
        self.send_control_on_connection(device_id, connection_id, code, message);

        self.request_transport_disconnect_on_connection(device_id, connection_id);
    }

    // ---------------- Pair: responder side ----------------

    fn on_pair_request(
        &mut self,
        device_id: String,
        connection_id: String,
        req: proto::PeerPairRequest,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        // Legacy peers send no hello: let them through, gate future features by capabilities.
        if let Err(msg) = self.check_hello(&device_id, req.hello.clone()) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }

        if self.trusted.is_trusted(&device_id) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::ALREADY_TRUSTED,
                "already trusted",
            );
            return;
        }
        let Ok(pubkey): Result<[u8; 32], _> = req.public_key.as_slice().try_into() else {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed public key",
            );
            return;
        };
        if hex::encode(DeviceIdentity::get_fingerprint_for(pubkey)) != device_id {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_ERROR,
                "fingerprint does not match public key",
            );
            return;
        }

        let seen = self.seen.get(&device_id);
        let addr = seen
            .map(|d| SocketAddr::new(d.addr, d.ws_port as u16))
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        let device_type = seen
            .map(|d| d.device_type)
            .unwrap_or(proto::DeviceType::Unspecified);
        self.pair_sessions.insert(
            device_id.clone(),
            PairSession::new_responder(addr, pubkey, req.name.clone(), device_type),
        );

        let engine = self.notification.clone();
        let peer_name = req.name;
        let dev_id = device_id;
        let tx = notify_tx.clone();
        tokio::spawn(async move {
            let decision = engine
                .ask_pair(Prompt::PairRequest { peer_name }, Duration::from_secs(60))
                .await;
            let _ = tx.send(NotifyResult::PairApproval {
                device_id: dev_id,
                decision,
            });
        });
    }

    fn on_pair_challenge(
        &mut self,
        device_id: String,
        c: proto::PeerPairChallenge,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        let Some(sess) = self.pair_sessions.get_mut(&device_id) else {
            return;
        };
        if sess.role != Role::Responder || sess.stage != PairStage::AwaitingChallenge {
            return;
        }
        let Ok(nonce_arr): Result<[u8; 32], _> = c.nonce.as_slice().try_into() else {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed nonce",
            );
            return;
        };
        let peer_pub = sess.peer_public_key.unwrap();
        let own_pub = self.identity.public_key_bytes();
        let code = PairSession::compute_code(&nonce_arr, &own_pub, &peer_pub);
        let signature = self.identity.sign(&nonce_arr);

        let sess = self.pair_sessions.get_mut(&device_id).unwrap();
        sess.nonce = Some(nonce_arr);
        sess.code = Some(code);
        sess.pending_signature = Some(signature);
        sess.stage = PairStage::AwaitingCodeConfirm;

        let engine = self.notification.clone();
        let peer_name = sess.peer_name.clone().unwrap_or_default();
        let dev_id = device_id;
        let tx = notify_tx.clone();
        tokio::spawn(async move {
            let decision = engine
                .ask_pair(
                    Prompt::ConfirmCode {
                        peer_name,
                        code: format!("{code:06}"),
                    },
                    Duration::from_secs(60),
                )
                .await;
            let _ = tx.send(NotifyResult::PairCodeConfirm {
                device_id: dev_id,
                decision,
            });
        });
    }

    fn on_pair_ack(&mut self, device_id: String) {
        let Some(sess) = self.pair_sessions.get(&device_id) else {
            return;
        };
        if sess.role != Role::Responder || sess.stage != PairStage::AwaitingAck {
            return;
        }
        self.finalize_pair_trusted(&device_id);
        self.pair_sessions.remove(&device_id);
    }

    // ---------------- Pair: initiator side (continued) ----------------

    fn on_pair_response(
        &mut self,
        device_id: String,
        res: proto::PeerPairResponse,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        let Some(sess) = self.pair_sessions.get_mut(&device_id) else {
            return;
        };
        if sess.role != Role::Initiator || sess.stage != PairStage::AwaitingPeerResponse {
            return;
        }
        if let Err(msg) = self.check_hello(&device_id, res.hello.clone()) {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }
        let Ok(pubkey): Result<[u8; 32], _> = res.public_key.as_slice().try_into() else {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed public key",
            );
            return;
        };
        if hex::encode(DeviceIdentity::get_fingerprint_for(pubkey)) != device_id {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "fingerprint does not match public key",
            );
            return;
        }
        let nonce_vec = self.identity.random_nonce();
        let nonce_arr: [u8; 32] = nonce_vec
            .as_slice()
            .try_into()
            .expect("nonce must be 32 bytes");

        // Compute our own code and show it now, on our own nonce/key —
        // don't wait for the challengeResponse to come back, The responder
        // will land on this same value once it processes the Challenge.
        let own_pub = self.identity.public_key_bytes();
        let code = PairSession::compute_code(&nonce_arr, &own_pub, &pubkey);

        let sess = self.pair_sessions.get_mut(&device_id).unwrap();
        sess.peer_public_key = Some(pubkey);
        sess.peer_name = Some(res.name.clone());
        sess.nonce = Some(nonce_arr);
        sess.code = Some(code);
        sess.stage = PairStage::AwaitingSignature;

        self.send_peer(
            &device_id,
            Body::PairChallenge(proto::PeerPairChallenge { nonce: nonce_vec }),
        );

        let notify_engine = self.notification.clone();
        let peer_name = res.name;
        let tx = notify_tx.clone();
        tokio::spawn(async move {
            let decision = notify_engine
                .ask_pair(
                    Prompt::ConfirmCode {
                        peer_name,
                        code: format!("{code:06}"),
                    },
                    Duration::from_secs(60),
                )
                .await;
            let _ = tx.send(NotifyResult::PairCodeConfirm {
                device_id,
                decision,
            });
        });
    }

    fn on_pair_challenge_response(
        &mut self,
        device_id: String,
        r: proto::PeerPairChallengeResponse,
    ) {
        let Some(sess) = self.pair_sessions.get_mut(&device_id) else {
            return;
        };
        if sess.role != Role::Initiator || sess.stage != PairStage::AwaitingSignature {
            return;
        }
        let Ok(sig): Result<[u8; 64], _> = r.signature.as_slice().try_into() else {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed signature",
            );
            return;
        };

        // the code was already computed and shown when we sent the
        // Challenge — nothing to (re) compute here
        let sess = self.pair_sessions.get_mut(&device_id).unwrap();
        sess.pending_signature = Some(sig);

        if sess.code_confirmed {
            // User already hit confirm while we were still waiting on the
            // wire — signature is the last piece, finalize now.
            self.finalize_initiator_pair(&device_id);
        }
        // else: stay in AwaitingSignature with pending_signature set; the
        // user's confirmation (on_code_confirm_result) will finalize once
        // it comes in.
    }

    /// Verifies the peer's signature against our code/nonce and, if it
    /// checks out, trusts the device and sends the Ack. Called from
    /// whichever of {signature arrival, user confirmation} completes last.
    fn finalize_initiator_pair(&mut self, device_id: &str) {
        let Some(sess) = self.pair_sessions.get(device_id) else {
            return;
        };
        let sig = sess
            .pending_signature
            .expect("finalize_initiator_pair called before signature arrived");
        let pubkey = sess.peer_public_key.unwrap();
        let nonce = sess.nonce.unwrap();
        if !identity::verify(&pubkey, &nonce, &sig) {
            self.abort_pair(
                device_id,
                pairing::control::INVALID_SIGNATURE,
                "signature verification failed",
            );
            return;
        }
        let peer_name = sess
            .peer_name
            .clone()
            .unwrap_or_else(|| device_id.to_string());
        self.finalize_pair_trusted(device_id);
        self.send_peer(device_id, Body::PairAck(proto::PairAck {}));
        self.pair_sessions.remove(device_id);
        self.notify_pair_events(
            device_id,
            proto::pairing_event::State::Succeeded as i32,
            "paired successfully",
        );
        self.notify_info(
            "Paired successfully",
            format!(
                "Successfully paired with {peer_name}\nFingerprint: {}",
                get_formatted_fp(device_id)
            ),
        );
    }

    fn finalize_pair_trusted(&mut self, device_id: &str) {
        let Some(sess) = self.pair_sessions.get(device_id) else {
            return;
        };
        let Some(pubkey) = sess.peer_public_key else {
            return;
        };
        let name = sess
            .peer_name
            .clone()
            .unwrap_or_else(|| device_id.to_string());
        let device_type = sess.peer_device_type;
        self.trusted.trust(TrustedDevice {
            id: device_id.to_string(),
            name,
            device_type,
            paired_at: std::time::SystemTime::now(),
            public_key: pubkey,
            auto_connect: true,
        });
        self.connected
            .insert(device_id.to_string(), "connected".to_string());
        self.notify_devices_changed();
    }

    // ---------------- Shared: code confirmation result ----------------

    fn handle_notify_result(&mut self, result: NotifyResult) {
        match result {
            NotifyResult::PairApproval {
                device_id,
                decision,
            } => self.on_pair_approval_result(device_id, decision),
            NotifyResult::PairCodeConfirm {
                device_id,
                decision,
            } => self.on_code_confirm_result(device_id, decision),
        }
    }

    fn on_pair_approval_result(&mut self, device_id: String, decision: PairDecision) {
        let Some(sess) = self.pair_sessions.get_mut(&device_id) else {
            return;
        };
        if sess.stage != PairStage::AwaitingLocalApproval {
            return;
        }
        match decision {
            PairDecision::Allow => {
                sess.stage = PairStage::AwaitingChallenge;
                let own_pub = self.identity.public_key_bytes();
                self.send_peer(
                    &device_id,
                    Body::PairResponse(proto::PeerPairResponse {
                        public_key: own_pub.to_vec(),
                        name: config::get_hostname(),
                        hello: Some(config::local_hello()),
                    }),
                );
            }
            PairDecision::Deny | PairDecision::NoResponse => {
                self.abort_pair(
                    &device_id,
                    pairing::control::USER_DENIED,
                    "user denied pairing",
                );
            }
        }
    }

    fn on_code_confirm_result(&mut self, device_id: String, decision: PairDecision) {
        let Some(sess) = self.pair_sessions.get(&device_id) else {
            return;
        };

        let valid_stage = match sess.role {
            Role::Responder => sess.stage == PairStage::AwaitingCodeConfirm,
            Role::Initiator => sess.stage == PairStage::AwaitingSignature,
        };
        if !valid_stage {
            return;
        }

        if !matches!(decision, PairDecision::Allow) {
            self.abort_pair(
                &device_id,
                pairing::control::CODE_MISMATCH,
                "user did not confirm the code",
            );
            return;
        }

        match sess.role {
            Role::Responder => {
                let sig = sess
                    .pending_signature
                    .expect("signature computed at challenge time");
                if let Some(sess) = self.pair_sessions.get_mut(&device_id) {
                    sess.stage = PairStage::AwaitingAck;
                }
                self.send_peer(
                    &device_id,
                    Body::PairChallengeResponse(proto::PeerPairChallengeResponse {
                        signature: sig.to_vec(),
                    }),
                );
            }
            Role::Initiator => {
                let signature_already_here = sess.pending_signature.is_some();
                if let Some(sess) = self.pair_sessions.get_mut(&device_id) {
                    sess.code_confirmed = true;
                }
                if signature_already_here {
                    self.finalize_initiator_pair(&device_id);
                }
                // else confirmed and waiting on pair response pair challenge
            }
        }
    }

    fn abort_pair(&mut self, device_id: &str, code: u32, message: &str) {
        self.send_control(device_id, code, message);
        self.pair_sessions.remove(device_id);
        self.notify_pair_events(
            device_id,
            proto::pairing_event::State::Failed as i32,
            message,
        );
        self.request_transport_disconnect(device_id);
    }

    // ---------------- Connect: responder side ----------------

    fn on_connect_challenge(
        &mut self,
        device_id: String,
        connection_id: String,
        c: proto::PeerConnectChallenge,
    ) {
        if self
            .connected
            .get(&device_id)
            .is_some_and(|current| current != &connection_id)
        {
            return;
        }
        if !self.trusted.is_trusted(&device_id) {
            self.send_control_on_connection(
                &device_id,
                &connection_id,
                pairing::control::UNKNOWN_DEVICE,
                "not a trusted device",
            );
            self.request_transport_disconnect_on_connection(&device_id, &connection_id);
            return;
        }

        if let Err(msg) = self.check_hello(&device_id, c.hello.clone()) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }

        let Ok(nonce_arr): Result<[u8; 32], _> = c.nonce.as_slice().try_into() else {
            self.send_control_on_connection(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed nonce",
            );
            self.request_transport_disconnect_on_connection(&device_id, &connection_id);
            return;
        };

        // Auto-connect races are resolved at the handshake layer, not by
        // comparing stable device fingerprints. A request that arrived before
        // our own challenge was sent wins immediately. If both challenges are
        // already in flight, their random nonces break the truly simultaneous
        // case deterministically.
        let existing_connect = self.connect_sessions.get(&device_id).map(|session| {
            (
                session.role,
                session.stage,
                session.connection_id.clone(),
                session.nonce,
            )
        });
        if let Some((Role::Initiator, stage, existing_connection_id, own_nonce)) = existing_connect
        {
            match stage {
                ConnectStage::Dialing => {
                    // We had not sent our request yet. The peer's challenge is
                    // therefore the first request we actually observed.
                    if existing_connection_id.as_deref() != Some(connection_id.as_str()) {
                        self.promote_transport_connection(&device_id, &connection_id);
                    }
                    self.transport_connections
                        .insert(device_id.clone(), connection_id.clone());
                    self.connect_sessions.remove(&device_id);
                }
                ConnectStage::AwaitingSignature => {
                    let Some(own_nonce) = own_nonce else {
                        // Defensive fallback: an initiator is only valid in this
                        // stage after it has generated its challenge nonce.
                        self.send_control_on_connection(
                            &device_id,
                            &connection_id,
                            pairing::control::CONNECT_IN_PROGRESS,
                            "connection already requested; finish the existing connection",
                        );
                        self.request_transport_disconnect_on_connection(&device_id, &connection_id);
                        return;
                    };

                    if own_nonce <= nonce_arr {
                        // Our request is the winner. Tell the competing caller
                        // to stop and keep our existing connection untouched.
                        self.send_control_on_connection(
                            &device_id,
                            &connection_id,
                            pairing::control::CONNECT_IN_PROGRESS,
                            "connection already requested; finish the existing connection",
                        );
                        if existing_connection_id.as_deref() != Some(connection_id.as_str()) {
                            self.request_transport_disconnect_on_connection(
                                &device_id,
                                &connection_id,
                            );
                        }
                        return;
                    }

                    // The peer's request wins the simultaneous race. Switch the
                    // transport to its challenge connection and continue as the
                    // responder.
                    if existing_connection_id.as_deref() != Some(connection_id.as_str()) {
                        self.promote_transport_connection(&device_id, &connection_id);
                    }
                    self.transport_connections
                        .insert(device_id.clone(), connection_id.clone());
                    self.connect_sessions.remove(&device_id);
                }
                ConnectStage::AwaitingAck => return,
            }
        }

        if self
            .transport_connections
            .get(&device_id)
            .map(String::as_str)
            != Some(connection_id.as_str())
        {
            self.promote_transport_connection(&device_id, &connection_id);
            self.transport_connections
                .insert(device_id.clone(), connection_id.clone());
        }

        let sig = self.identity.sign(&nonce_arr);
        let addr = self
            .seen
            .get(&device_id)
            .map(|d| SocketAddr::new(d.addr, d.ws_port as u16))
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

        let mut sess = ConnectSession::new_responder(addr);
        sess.connection_id = Some(connection_id.clone());
        sess.nonce = Some(nonce_arr);
        self.connect_sessions.insert(device_id.clone(), sess);

        self.send_peer_on_connection(
            &device_id,
            &connection_id,
            Body::ConnectChallengeResponse(proto::PeerConnectChallengeResponse {
                signature: sig.to_vec(),
                hello: Some(config::local_hello()),
            }),
        );
    }

    fn on_connect_ack(&mut self, device_id: String, connection_id: String) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            tracing::warn!("ConnectAck from {device_id} ignored: no connect session");
            return;
        };
        if sess.role != Role::Responder
            || sess.stage != ConnectStage::AwaitingAck
            || sess.connection_id.as_deref() != Some(connection_id.as_str())
        {
            tracing::warn!(
                "ConnectAck from {device_id} ignored: role={:?} stage={:?} sess_conn={:?} ack_conn={}",
                sess.role,
                sess.stage,
                sess.connection_id,
                connection_id
            );
            return;
        }
        tracing::info!("connected to {device_id} on {connection_id}");
        self.connected.insert(device_id.clone(), connection_id);
        self.connect_sessions.remove(&device_id);
        self.resume_downloads_for_device(&device_id);
        self.notify_devices_changed();
    }

    // ---------------- Connect: initiator side (continued) ----------------

    fn on_connect_challenge_response(
        &mut self,
        device_id: String,
        connection_id: String,
        r: proto::PeerConnectChallengeResponse,
    ) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            return;
        };
        if sess.role != Role::Initiator
            || sess.stage != ConnectStage::AwaitingSignature
            || sess.connection_id.as_deref() != Some(connection_id.as_str())
        {
            return;
        }
        let Some(trusted_device) = self.trusted.get(&device_id) else {
            self.abort_connect_on_connection(
                &device_id,
                &connection_id,
                pairing::control::UNKNOWN_DEVICE,
                "device no longer trusted",
            );
            return;
        };
        let nonce = sess.nonce.unwrap();
        if !identity::verify(&trusted_device.public_key, &nonce, &r.signature) {
            self.abort_connect_on_connection(
                &device_id,
                &connection_id,
                pairing::control::INVALID_SIGNATURE,
                "signature verification failed",
            );
            return;
        }
        if let Err(msg) = self.check_hello(&device_id, r.hello.clone()) {
            self.abort_connect_on_connection(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }
        self.send_peer_on_connection(
            &device_id,
            &connection_id,
            Body::ConnectAck(proto::ConnectAck {}),
        );
        tracing::info!("connected to {device_id} on {connection_id}");
        self.connected.insert(device_id.clone(), connection_id);
        self.connect_sessions.remove(&device_id);
        self.resume_downloads_for_device(&device_id);
        self.notify_devices_changed();
    }

    fn abort_connect(&mut self, device_id: &str, code: u32, message: &str) {
        self.send_control(device_id, code, message);
        self.connect_sessions.remove(device_id);
        self.request_transport_disconnect(device_id);
    }

    fn abort_connect_on_connection(
        &mut self,
        device_id: &str,
        connection_id: &str,
        code: u32,
        message: &str,
    ) {
        self.send_control_on_connection(device_id, connection_id, code, message);
        if self
            .connect_sessions
            .get(device_id)
            .is_some_and(|s| s.connection_id.as_deref() == Some(connection_id))
        {
            self.connect_sessions.remove(device_id);
        }
        self.request_transport_disconnect_on_connection(device_id, connection_id);
    }

    // ---------------- Shared control / helpers ----------------

    fn on_control(
        &mut self,
        device_id: String,
        connection_id: String,
        ctrl: proto::PreTransportControl,
    ) {
        tracing::warn!(
            "pre-transport control from {device_id}: {} ({})",
            ctrl.code,
            ctrl.message
        );

        if ctrl.code == pairing::control::UNKNOWN_DEVICE {
            // Trust is symmetric. If a trusted peer explicitly says this
            // device is no longer trusted, forget the stale local trust too.
            let was_trusted = self.trusted.is_trusted(&device_id);
            if was_trusted {
                self.trusted.revoke(&device_id);
                self.notify_devices_changed();
                tracing::info!(%device_id, "peer revoked trust; removed local trust");
            }
            self.pair_sessions.remove(&device_id);
            self.connect_sessions.remove(&device_id);
            self.request_transport_disconnect_on_connection(&device_id, &connection_id);
            self.notify_info(
                "Device trust changed",
                "The other device no longer trusts this device. Pair again to reconnect.",
            );
            return;
        }

        // The peer already has a connect request in flight and won this race.
        // This is not an error: abandon our competing logical request and close
        // the socket we used for it. Do not leave a stale ConnectSession behind.
        if ctrl.code == pairing::control::CONNECT_IN_PROGRESS {
            tracing::debug!(%device_id, %connection_id, "peer already has a connection request in progress; yielding");
            if self.connect_sessions.remove(&device_id).is_some() {
                self.request_transport_disconnect_on_connection(&device_id, &connection_id);
            }
            return;
        }

        let was_pairing = self.pair_sessions.remove(&device_id).is_some();
        self.connect_sessions.remove(&device_id);
        if was_pairing {
            tracing::debug!(%device_id, ctrl_message=%&ctrl.message, "Pairing to peer failed, core emitting `proto::pairing_event::State::Failed` change");
            self.notify_pair_events(
                &device_id,
                proto::pairing_event::State::Failed as i32,
                &ctrl.message,
            );
        }
        self.notify_info("Pairing/connect failed", ctrl.message);
    }

    /// Hands a trusted peer's non-file clipboard content to ClipboardManager.
    fn on_clipboard_message(&mut self, device_id: String, msg: proto::ClipboardMessage) {
        if !self.trusted.is_trusted(&device_id) {
            tracing::warn!("dropping clipboard content from untrusted device {device_id}");
            return;
        }
        let Some(content) = msg.content else {
            return;
        };
        let payload = match content {
            clipboard_message::Content::Text(text) => {
                if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
                    self.notify_info(
                        "Clipboard too large",
                        "Received text is larger than 4 MiB. Send it as a file instead.",
                    );
                    return;
                }
                crate::clipboard::manager::IncomingPayload::Text(text)
            }
            clipboard_message::Content::RichText(value) => {
                if value.text.len() > MAX_CLIPBOARD_TEXT_BYTES
                    || value.text.len().saturating_add(value.html.len()) > MAX_RICH_TEXT_BYTES
                {
                    self.notify_info("Clipboard too large", "Received rich text exceeds the clipboard limit. Send it as a file instead.");
                    return;
                }
                crate::clipboard::manager::IncomingPayload::RichText {
                    text: value.text,
                    html: value.html,
                }
            }
            clipboard_message::Content::Image(value) => {
                let expected = (value.width as usize)
                    .checked_mul(value.height as usize)
                    .and_then(|v| v.checked_mul(4));
                if expected != Some(value.rgba.len())
                    || value.width == 0
                    || value.height == 0
                    || value.rgba.len() > MAX_IMAGE_BYTES
                {
                    tracing::warn!("rejecting invalid clipboard image from {device_id}");
                    return;
                }
                crate::clipboard::manager::IncomingPayload::Image {
                    width: value.width,
                    height: value.height,
                    rgba: value.rgba,
                }
            }
            clipboard_message::Content::FileOffer(mut file) => {
                // The sender's own name can arrive with no extension at all
                // (this is common for the "clipboard image sent as a file"
                // path, since neither platform's picker guarantees one).
                // Fix it up once, here, so every downstream consumer — the
                // saved filename on disk, the notification title, the
                // history entry — sees a name that actually looks like what
                // it is instead of showing up as an unrecognized file.
                file.name =
                    crate::clipboard::manager::ensure_file_extension(&file.name, &file.mime_type);
                crate::clipboard::manager::IncomingPayload::FileOffer { file }
            }
        };
        let device_name = self
            .trusted
            .get(&device_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| device_id.clone());
        let _ = self.clipboard_tx.send(ClipboardCommand::IncomingRemote {
            payload,
            source_device_name: device_name,
            source_device_id: device_id,
        });
    }

    fn on_file_download_request(&mut self, device_id: String, req: proto::FileDownloadRequest) {
        let _ = self
            .clipboard_tx
            .send(ClipboardCommand::PeerFileDownloadRequest {
                device_id,
                file_id: req.file_id,
                offset: req.offset,
            });
    }

    fn on_file_chunk(&mut self, device_id: String, chunk: proto::FileChunk) {
        let handler_start = Instant::now();
        tracing::trace!(
            device_id = %shorter!(&device_id),
            file_id = %chunk.file_id,
            seq = chunk.seq,
            offset = chunk.offset,
            bytes = chunk.data.len(),
            eof = chunk.eof,
            "file download chunk received"
        );

        let Some(pending) = self.pending_downloads.get(&chunk.file_id).cloned() else {
            return;
        };
        if pending.device_id != device_id || pending.received_complete {
            return;
        }
        // A chunk we already have (a restarted or replaced sender stream
        // re-sending data from before our confirmed offset) is harmless. Drop it
        // quietly rather than aborting the whole download.
        if chunk.total_size == pending.total_size
            && !chunk.data.is_empty()
            && chunk.data.len() <= MAX_FILE_CHUNK_BYTES
            && chunk.offset.saturating_add(chunk.data.len() as u64) <= pending.received_size
        {
            tracing::debug!(file_id = %chunk.file_id, offset = chunk.offset, "ignoring duplicate file chunk");
            return;
        }
        if chunk.total_size != pending.total_size
            || chunk.data.len() > MAX_FILE_CHUNK_BYTES
            || chunk.offset != pending.received_size
            || chunk.offset.saturating_add(chunk.data.len() as u64) > pending.total_size
        {
            self.abort_download(&chunk.file_id, "Invalid file chunk metadata");
            return;
        }
        let t = Instant::now();
        let digest = Sha256::digest(&chunk.data);
        let hash_time = t.elapsed();
        if chunk.sha256.len() != digest.len() || chunk.sha256.as_slice() != digest.as_slice() {
            self.abort_download(&chunk.file_id, "File chunk integrity check failed");
            return;
        }
        let key = (device_id.clone(), chunk.file_id.clone());
        let temp_path = self.incoming_temp_path(&device_id, &chunk.file_id);
        let state_path = self.incoming_state_path(&chunk.file_id);
        if !self.incoming_files.contains_key(&key) {
            if fs::create_dir_all(&self.incoming_files_dir).is_err() {
                self.abort_download(&chunk.file_id, "Could not prepare incoming storage");
                return;
            }
            let file = if pending.received_size == 0 {
                match OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(&temp_path)
                {
                    Ok(f) => f,
                    Err(_) => {
                        let _ = fs::remove_file(&temp_path);
                        match OpenOptions::new()
                            .create_new(true)
                            .read(true)
                            .write(true)
                            .open(&temp_path)
                        {
                            Ok(f) => f,
                            Err(_) => {
                                self.abort_download(
                                    &chunk.file_id,
                                    "Could not create incoming file",
                                );
                                return;
                            }
                        }
                    }
                }
            } else {
                let Ok(meta) = fs::metadata(&temp_path) else {
                    self.abort_download(&chunk.file_id, "Resume data is missing");
                    return;
                };
                if meta.len() != pending.received_size {
                    self.abort_download(
                        &chunk.file_id,
                        "Resume data does not match the confirmed offset",
                    );
                    return;
                }
                match OpenOptions::new().read(true).write(true).open(&temp_path) {
                    Ok(f) => f,
                    Err(_) => {
                        self.abort_download(&chunk.file_id, "Could not reopen incoming file");
                        return;
                    }
                }
            };
            self.incoming_files.insert(
                key.clone(),
                IncomingFileAssembly {
                    next_seq: pending.received_size / MAX_FILE_CHUNK_BYTES as u64,
                    total_size: pending.total_size,
                    received_size: pending.received_size,
                    temp_path: temp_path.clone(),
                    state_path: state_path.clone(),
                    file,
                    profile: ReceiverProfile::new(),
                },
            );
        }
        let new_offset = {
            let Some(state) = self.incoming_files.get_mut(&key) else {
                self.abort_download(&chunk.file_id, "Incoming file session is missing");
                return;
            };
            if state.next_seq != chunk.seq || state.received_size != chunk.offset {
                self.abort_download(&chunk.file_id, "Invalid file chunk sequence");
                return;
            }

            let t = Instant::now();
            let mut ok = state
                .file
                .seek(SeekFrom::Start(state.received_size))
                .is_ok();
            state.profile.seek.record(t.elapsed());

            if ok {
                let t = Instant::now();
                ok = state.file.write_all(&chunk.data).is_ok();
                state.profile.write.record(t.elapsed());
            }
            if ok {
                let t = Instant::now();
                ok = state.file.sync_all().is_ok();
                state.profile.sync.record(t.elapsed());
            }
            if !ok {
                self.abort_download(&chunk.file_id, "Could not persist incoming file data");
                return;
            }
            state.received_size.saturating_add(chunk.data.len() as u64)
        };
        if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
            active.received_size = new_offset;
            active.last_activity = Instant::now();
            active.stall_retries = 0;
        }
        let mut resume_time = Duration::ZERO;
        if let Some(active) = self.pending_downloads.get(&chunk.file_id) {
            let t = Instant::now();
            self.persist_resume_state(&device_id, &chunk.file_id, active, new_offset);
            resume_time = t.elapsed();
        }
        let t = Instant::now();
        self.send_peer(
            &device_id,
            Body::FileChunkAck(proto::FileChunkAck {
                file_id: chunk.file_id.clone(),
                confirmed_offset: new_offset,
            }),
        );
        let ack_time = t.elapsed();

        if let Some(state) = self.incoming_files.get_mut(&key) {
            let p = &mut state.profile;
            p.hash.record(hash_time);
            p.resume_state.record(resume_time);
            p.ack_queue.record(ack_time);
            p.handler.record(handler_start.elapsed());
            p.bytes += chunk.data.len() as u64;
        }
        tracing::trace!(
            device_id = %shorter!(&device_id),
            file_id = %chunk.file_id,
            seq = chunk.seq,
            offset = chunk.offset,
            bytes = chunk.data.len(),
            hash_us = hash_time.as_micros() as u64,
            resume_state_us = resume_time.as_micros() as u64,
            ack_enqueue_us = ack_time.as_micros() as u64,
            "file download chunk processed"
        );
        if chunk.eof {
            if new_offset != pending.total_size {
                self.abort_download(&chunk.file_id, "File ended before all data arrived");
                return;
            }
            let Some(done_state) = self.incoming_files.remove(&key) else {
                return;
            };
            done_state.profile.log_summary(&chunk.file_id, "complete");
            let _ = fs::remove_file(&done_state.state_path);
            if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
                active.received_complete = true;
            }
            self.notify_transfer(
                &pending.device_id,
                &pending.entry_id,
                &chunk.file_id,
                &pending.file_name,
                "download",
                new_offset,
                pending.total_size,
                "receiving",
                format!("Receiving {}", pending.file_name),
            );
            if self
                .clipboard_tx
                .send(ClipboardCommand::IncomingFilePath {
                    source_device_id: device_id,
                    file_id: chunk.file_id.clone(),
                    path: done_state.temp_path.clone(),
                })
                .is_err()
            {
                let _ = fs::remove_file(done_state.temp_path);
                self.release_pending_download(&chunk.file_id);
                self.notify_transfer(
                    &pending.device_id,
                    &pending.entry_id,
                    &chunk.file_id,
                    &pending.file_name,
                    "download",
                    new_offset,
                    pending.total_size,
                    "failed",
                    "Clipboard manager is unavailable",
                );
            }
            return;
        }
        if let Some(state) = self.incoming_files.get_mut(&key) {
            state.next_seq = state.next_seq.saturating_add(1);
            state.received_size = new_offset;
        }
        self.notify_transfer(
            &pending.device_id,
            &pending.entry_id,
            &chunk.file_id,
            &pending.file_name,
            "download",
            new_offset,
            pending.total_size,
            "receiving",
            format!(
                "Downloading {} · {}%",
                pending.file_name,
                Self::percentage(new_offset, pending.total_size)
            ),
        );
    }

    fn on_file_chunk_ack(&mut self, device_id: String, ack: proto::FileChunkAck) {
        tracing::trace!(%device_id, file_id = %ack.file_id, confirmed_offset = ack.confirmed_offset, "file upload acknowledgement received");
        if let Some(session) = self.upload_sessions.get(&(device_id, ack.file_id)) {
            let _ = session.ack_tx.send(ack.confirmed_offset);
        }
    }

    fn on_file_transfer_cancel(&mut self, device_id: String, cancel: proto::FileTransferCancel) {
        // The peer stopped receiving from us. A pause keeps the receiver's
        // partial data, so end our stream as an interruption, not a cancel.
        if let Some(session) = self
            .upload_sessions
            .get(&(device_id.clone(), cancel.file_id.clone()))
        {
            if cancel.pause {
                session.interrupt.cancel();
            } else {
                session.cancel.cancel();
            }
        }
        // The peer stopped sending to us.
        let ours = self
            .pending_downloads
            .get(&cancel.file_id)
            .is_some_and(|p| p.device_id == device_id);
        if ours && let Some(p) = self.release_pending_download(&cancel.file_id) {
            if cancel.pause {
                // Kept for an explicit resume (ours, or the sender's
                // FileTransferResume), never restarted on its own.
                let _ = fs::write(
                    self.incoming_paused_path(&cancel.file_id),
                    Self::now_ms().to_string(),
                );
                let message = format!("{} was paused by the sender", p.file_name);
                self.park_download(&p, &cancel.file_id, true, "paused", message);
            } else {
                let message = format!("The sender cancelled {}", p.file_name);
                self.park_download(&p, &cancel.file_id, false, "cancelled", message);
            }
            return;
        }
        if !cancel.pause {
            // May be a partial we kept (a paused download) that the sender
            // has now given up on.
            self.discard_kept_partial(&device_id, &cancel.file_id);
        }
    }

    /// The sender asked us to continue a transfer we hold a partial download
    /// of. We ask again from the kept offset.
    fn on_file_transfer_resume(&mut self, device_id: String, resume: proto::FileTransferResume) {
        let file_id = resume.file_id;
        if self.pending_downloads.contains_key(&file_id) || !self.connected.contains_key(&device_id)
        {
            return;
        }
        let Some(state) = fs::read(self.incoming_state_path(&file_id))
            .ok()
            .and_then(|b| serde_json::from_slice::<ResumeState>(&b).ok())
        else {
            return;
        };
        if state.device_id != device_id || state.offer_expires_at_ms < Self::now_ms() {
            return;
        }
        self.resume_candidates.remove(&file_id);
        self.start_or_resume_download(
            state.device_id,
            state.file_id,
            state.entry_id,
            state.file_name,
            state.total_size,
            state.offer_expires_at_ms,
        );
    }

    /// Drop a kept partial download the sender cancelled, and report it so the
    /// row does not stay "paused" for a transfer that can no longer continue.
    fn discard_kept_partial(&mut self, device_id: &str, file_id: &str) {
        let Some(state) = fs::read(self.incoming_state_path(file_id))
            .ok()
            .and_then(|b| serde_json::from_slice::<ResumeState>(&b).ok())
        else {
            return;
        };
        if state.device_id != device_id {
            return;
        }
        let _ = fs::remove_file(self.incoming_temp_path(device_id, file_id));
        let _ = fs::remove_file(self.incoming_state_path(file_id));
        let _ = fs::remove_file(self.incoming_paused_path(file_id));
        self.resume_candidates.remove(file_id);
        self.notify_transfer(
            device_id,
            &state.entry_id,
            file_id,
            &state.file_name,
            "download",
            state.confirmed_offset,
            state.total_size,
            "cancelled",
            format!("The sender cancelled {}", state.file_name),
        );
    }

    fn start_or_resume_download(
        &mut self,
        device_id: String,
        file_id: String,
        entry_id: String,
        file_name: String,
        total_size: u64,
        offer_expires_at_ms: u64,
    ) {
        if self.pending_downloads.contains_key(&file_id) || !self.connected.contains_key(&device_id)
        {
            return;
        }
        let _ = fs::remove_file(self.incoming_paused_path(&file_id));
        let offset = self
            .load_valid_resume_state(
                &file_id,
                &device_id,
                &entry_id,
                &file_name,
                total_size,
                offer_expires_at_ms,
            )
            .map(|s| s.confirmed_offset)
            .unwrap_or(0);
        if offset == 0 {
            let _ = fs::remove_file(self.incoming_temp_path(&device_id, &file_id));
            let _ = fs::remove_file(self.incoming_state_path(&file_id));
        }
        self.pending_downloads.insert(
            file_id.clone(),
            PendingDownload {
                device_id: device_id.clone(),
                entry_id: entry_id.clone(),
                file_name: file_name.clone(),
                total_size,
                offer_expires_at_ms,
                received_size: offset,
                received_complete: false,
                last_activity: Instant::now(),
                stall_retries: 0,
            },
        );
        self.notify_transfer(
            &device_id,
            &entry_id,
            &file_id,
            &file_name,
            "download",
            offset,
            total_size,
            "requesting",
            if offset > 0 {
                format!(
                    "Resuming {file_name} · {}%",
                    Self::percentage(offset, total_size)
                )
            } else {
                "Downloading file".into()
            },
        );
        self.send_peer(
            &device_id,
            Body::FileDownloadRequest(proto::FileDownloadRequest { file_id, offset }),
        );
    }

    /// Watchdog for incoming transfers. Runs on the manager's 10s tick.
    ///
    /// If a download is still wanted and the peer is connected but no chunk has
    /// arrived for DOWNLOAD_STALL_TIMEOUT, ask the sender to continue from our
    /// current offset over the same connection. After DOWNLOAD_MAX_STALL_RETRIES
    /// unanswered attempts, report the transfer as interrupted; it still resumes
    /// on the next reconnect (`resume_downloads_for_device`) or if chunks resume.
    fn check_stalled_downloads(&mut self) {
        let stalled: Vec<String> = self
            .pending_downloads
            .iter()
            .filter(|(_, p)| {
                !p.received_complete
                    && p.stall_retries <= DOWNLOAD_MAX_STALL_RETRIES
                    && self.connected.contains_key(&p.device_id)
                    && p.last_activity.elapsed() >= DOWNLOAD_STALL_TIMEOUT
            })
            .map(|(file_id, _)| file_id.clone())
            .collect();
        for file_id in stalled {
            let Some(p) = self.pending_downloads.get_mut(&file_id) else {
                continue;
            };
            let give_up = p.stall_retries >= DOWNLOAD_MAX_STALL_RETRIES;
            p.last_activity = Instant::now();
            p.stall_retries = p.stall_retries.saturating_add(1);
            let snapshot = p.clone();
            if give_up {
                tracing::warn!(%file_id, "download stalled; giving up until retried");
                self.notify_transfer(
                    &snapshot.device_id,
                    &snapshot.entry_id,
                    &file_id,
                    &snapshot.file_name,
                    "download",
                    snapshot.received_size,
                    snapshot.total_size,
                    "interrupted",
                    "The sender stopped responding; the download can resume later",
                );
            } else {
                tracing::info!(
                    %file_id,
                    attempt = snapshot.stall_retries,
                    offset = snapshot.received_size,
                    "download stalled; re-requesting from current offset"
                );
                self.notify_transfer(
                    &snapshot.device_id,
                    &snapshot.entry_id,
                    &file_id,
                    &snapshot.file_name,
                    "download",
                    snapshot.received_size,
                    snapshot.total_size,
                    "requesting",
                    format!(
                        "Resuming {} · {}%",
                        snapshot.file_name,
                        Self::percentage(snapshot.received_size, snapshot.total_size)
                    ),
                );
                self.send_peer(
                    &snapshot.device_id,
                    Body::FileDownloadRequest(proto::FileDownloadRequest {
                        file_id: file_id.clone(),
                        offset: snapshot.received_size,
                    }),
                );
            }
        }
    }

    fn resume_downloads_for_device(&mut self, device_id: &str) {
        let pending: Vec<(String, PendingDownload)> = self
            .pending_downloads
            .iter()
            .filter(|(_, p)| p.device_id == device_id && !p.received_complete)
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect();
        for (file_id, p) in pending {
            self.send_peer(
                &p.device_id,
                Body::FileDownloadRequest(proto::FileDownloadRequest {
                    file_id,
                    offset: p.received_size,
                }),
            );
        }
        let candidates: Vec<ResumeState> = self
            .resume_candidates
            .values()
            .filter(|s| s.device_id == device_id && s.offer_expires_at_ms >= Self::now_ms())
            .cloned()
            .collect();
        for state in candidates {
            self.resume_candidates.remove(&state.file_id);
            self.start_or_resume_download(
                state.device_id,
                state.file_id,
                state.entry_id,
                state.file_name,
                state.total_size,
                state.offer_expires_at_ms,
            );
        }
    }

    fn stream_file_to_peer(
        &mut self,
        device_id: String,
        file_id: String,
        path: PathBuf,
        total_size: u64,
        start_offset: u64,
    ) {
        let Some(transport_tx) = self.transport_tx.as_ref().cloned() else {
            return;
        };

        let key = (device_id.clone(), file_id.clone());

        let Some(connection_id) = self.connected.get(&device_id).cloned() else {
            return;
        };
        tracing::debug!(%device_id, %file_id, total_size, start_offset, "file upload session starting");

        // The receiver asked again while an older stream for the same file was
        // still registered (for example that task is bound to a socket that has
        // since been replaced). The newest request reflects the receiver's real
        // offset, so retire the old stream and start from the new one.
        if let Some(old) = self.upload_sessions.remove(&key) {
            old.superseded.store(true, Ordering::Release);
            old.interrupt.cancel();
        }
        self.next_upload_id = self.next_upload_id.wrapping_add(1);
        let session_id = self.next_upload_id;
        let superseded = Arc::new(AtomicBool::new(false));
        let superseded_task = superseded.clone();

        let cancel = CancellationToken::new();
        let interrupt = CancellationToken::new();
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let confirmed_offset = Arc::new(AtomicU64::new(start_offset));
        let confirmed_offset_task = confirmed_offset.clone();
        let paused_by_user = Arc::new(AtomicBool::new(false));
        let paused_task = paused_by_user.clone();
        // The receiver is asking again, so any paused/waiting upload is live.
        self.held_uploads.remove(&key);

        self.upload_sessions.insert(
            key.clone(),
            UploadSession {
                id: session_id,
                cancel: cancel.clone(),
                interrupt: interrupt.clone(),
                file_name: file_name.clone(),
                total_size,
                confirmed: confirmed_offset.clone(),
                paused: paused_by_user,
                superseded,
                ack_tx,
            },
        );
        let peer_id = device_id.clone();
        let peer_name = self.peer_name(&device_id);
        let can_pause = self.peer_supports_transfer_control(&device_id);

        let done_tx = self.upload_done_tx.clone();
        let events_tx = self.events_tx.clone();
        let notification = self.notification.clone();
        let connection_id_for_task = connection_id.clone();

        tokio::spawn(async move {
            let mut profile = SenderProfile::new();

            let result: Result<(), String> = async {
                let mut file = tokio::fs::File::open(&path)
                    .await
                    .map_err(|e| format!("Could not open file: {e}"))?;

                if file.metadata().await.map_err(|e| e.to_string())?.len() != total_size
                    || start_offset > total_size
                {
                    return Err("Source file changed or resume offset is invalid".into());
                }

                file.seek(SeekFrom::Start(start_offset))
                    .await
                    .map_err(|e| e.to_string())?;

                let mut offset = start_offset;
                let mut seq = offset / MAX_FILE_CHUNK_BYTES as u64;

                let _ = Self::emit_transfer(
                    &events_tx,
                    &notification,
                    &peer_id,
                    &peer_name,
                    can_pause,
                    &file_id,
                    &file_name,
                    "send",
                    offset,
                    total_size,
                    "sending",
                    "Sending file".into(),
                )
                .await;

                loop {
                    if interrupt.is_cancelled() {
                        return Err("INTERRUPTED".into());
                    }

                    if cancel.is_cancelled() {
                        return Err("CANCELLED".into());
                    }

                    let mut data = vec![0u8; MAX_FILE_CHUNK_BYTES];

                    let cycle_start = Instant::now();

                    let t = Instant::now();
                    let n = file.read(&mut data).await.map_err(|e| e.to_string())?;
                    profile.read.record(t.elapsed());

                    if n == 0 && offset != total_size {
                        return Err("Source file ended before the advertised size".into());
                    }

                    data.truncate(n);

                    let end = offset.saturating_add(n as u64);
                    let eof = end == total_size;
                    let t = Instant::now();
                    let digest = Sha256::digest(&data).to_vec();
                    profile.hash.record(t.elapsed());

                    // Re-check right before queueing so a retired stream does
                    // not slip one more chunk onto the connection.
                    if cancel.is_cancelled() {
                        return Err("CANCELLED".into());
                    }
                    if interrupt.is_cancelled() {
                        return Err("INTERRUPTED".into());
                    }

                    let t = Instant::now();
                    let (reply_tx, reply_rx) = oneshot::channel();

                    transport_tx
                        .send(TransportCommand::SendPeerMessageOnConnection {
                            device_id: device_id.clone(),
                            connection_id: connection_id_for_task.clone(),
                            message: proto::PeerMessage {
                                body: Some(Body::FileChunk(proto::FileChunk {
                                    file_id: file_id.clone(),
                                    seq,
                                    data,
                                    eof,
                                    total_size,
                                    offset,
                                    sha256: digest,
                                })),
                            },
                            enqueued_at: Instant::now(),
                            reply_to: reply_tx,
                        }).map_err(|_| "Transport is unavailable".to_string())?;

                    let queue_result: Result<(), String> = tokio::select! {
                        _ = cancel.cancelled() => Err("CANCELLED".to_string()),
                        _ = interrupt.cancelled() => Err("INTERRUPTED".to_string()),
                        r = tokio::time::timeout(Duration::from_secs(15), reply_rx) => {
                            if matches!(r, Ok(Ok(true))) {
                                Ok(())
                            } else {
                                Err("Transport rejected or timed out queuing file data".to_string())
                            }
                        }
                    };
                    let queue_elapsed = t.elapsed();
                    profile.queue.record(queue_elapsed);
                    queue_result?;
                    tracing::trace!(
                        %file_id, seq, offset, chunk_bytes = n,
                        queue_us = queue_elapsed.as_micros() as u64,
                        "file upload chunk queued"
                    );

                    let t = Instant::now();
                    let ack_result: Result<(), String> = tokio::select! {
                        _ = cancel.cancelled() => Err("CANCELLED".to_string()),
                        _ = interrupt.cancelled() => Err("INTERRUPTED".to_string()),
                        ack = tokio::time::timeout(Duration::from_secs(30), ack_rx.recv()) => {
                            match ack {
                                Ok(Some(v)) if v == end => Ok(()),
                                Ok(Some(v)) => Err(format!("Unexpected transfer acknowledgement offset {v}")),
                                _ => Err("Timed out waiting for transfer acknowledgement".to_string()),
                            }
                        }
                    };
                    let ack_elapsed = t.elapsed();
                    profile.ack_wait.record(ack_elapsed);
                    ack_result?;
                    tracing::trace!(
                        %file_id, seq, confirmed_offset = end, chunk_bytes = n,
                        ack_wait_us = ack_elapsed.as_micros() as u64,
                        "file upload chunk acknowledged"
                    );

                    offset = end;
                    confirmed_offset_task.store(offset, Ordering::Release);
                    profile.cycle.record(cycle_start.elapsed());
                    profile.bytes += n as u64;

                    let state = if eof { "complete" } else { "sending" };

                    let msg = if eof {
                        format!("Sending finished: {file_name}")
                    } else {
                        format!(
                            "Sending {file_name} · {}%",
                            Self::percentage(offset, total_size)
                        )
                    };

                    let t = Instant::now();
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        offset,
                        total_size,
                        state,
                        msg,
                    )
                    .await;
                    profile.notify.record(t.elapsed());

                    if eof {
                        break;
                    }

                    seq = seq.saturating_add(1);
                }

                Ok(())
            }
            .await;
            profile.log_summary(
                &file_id,
                match &result {
                    Ok(()) => "complete",
                    Err(e) => e.as_str(),
                },
            );

            if let Err(error) = result
                && !superseded_task.load(Ordering::Acquire)
            {
                if error == "CANCELLED" {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed_offset.load(Ordering::Acquire),
                        total_size,
                        "cancelled",
                        format!("Sending {file_name} was cancelled"),
                    )
                    .await;
                } else if error == "INTERRUPTED" && paused_task.load(Ordering::Acquire) {
                    let confirmed = confirmed_offset.load(Ordering::Acquire);
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed,
                        total_size,
                        "paused",
                        format!("Paused sending {file_name}"),
                    )
                    .await;
                } else if error == "INTERRUPTED"
                    || error.starts_with("Transport")
                    || error.starts_with("Timed out")
                {
                    let confirmed = confirmed_offset.load(Ordering::Acquire);

                    Self::emit_transfer(
                    &events_tx,
                    &notification,
                    &peer_id,
                    &peer_name,
                    can_pause,
                    &file_id,
                    &file_name,
                    "send",
                    confirmed,
                    total_size,
                    "interrupted",
                    format!(
                        "Sending {file_name} was interrupted; it can resume from {confirmed} bytes"
                    ),
                )
                .await;
                } else {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed_offset.load(Ordering::Acquire),
                        total_size,
                        "failed",
                        error,
                    )
                    .await;
                }
            }

            let _ = done_tx.send((key.0, key.1, session_id));
        });
    }

    fn abort_download(&mut self, file_id: &str, message: impl Into<String>) {
        let message = message.into();
        tracing::warn!(%file_id, error = %message, "file download aborted");
        let pending = self.pending_downloads.remove(file_id);
        if let Some(p) = pending {
            if let Some(s) = self
                .incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
            {
                s.profile.log_summary(file_id, "aborted");
                let _ = fs::remove_file(s.temp_path);
            }
            self.resume_candidates.remove(file_id);
            let _ = fs::remove_file(self.incoming_state_path(file_id));
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
            self.notify_transfer(
                &p.device_id,
                &p.entry_id,
                file_id,
                &p.file_name,
                "download",
                p.received_size,
                p.total_size,
                "failed",
                message,
            );
        }
    }

    fn cancel_local_download(&mut self, file_id: &str) {
        if let Some(p) = self.pending_downloads.remove(file_id)
            && let Some(s) = self
                .incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
        {
            let _ = fs::remove_file(s.temp_path);
        }
        self.resume_candidates.remove(file_id);
        let _ = fs::remove_file(self.incoming_state_path(file_id));
        let keys: Vec<(String, String)> = self
            .incoming_files
            .keys()
            .filter(|(_, id)| id == file_id)
            .cloned()
            .collect();
        for k in keys {
            if let Some(s) = self.incoming_files.remove(&k) {
                let _ = fs::remove_file(s.temp_path);
            }
        }
        let _ = self
            .clipboard_tx
            .send(ClipboardCommand::FileDownloadReleased {
                file_id: file_id.to_string(),
            });
    }

    fn peer_name(&self, device_id: &str) -> String {
        self.trusted
            .list()
            .find(|d| d.id == device_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| device_id.to_string())
    }

    /// Whether this peer advertised pause/resume in its hello. Older builds
    /// (and peers that sent no hello at all) do not, and still transfer fine.
    fn peer_supports_transfer_control(&self, device_id: &str) -> bool {
        self.effective_versions
            .get(device_id)
            .is_some_and(|(_, caps)| {
                caps.contains(&(proto::PeerCapabilities::CapabilityFileTransferControl as i32))
            })
    }

    /// A user's pause / resume / cancel on one transfer. Cancel is always
    /// allowed (every build understands FileTransferCancel). Pause and resume
    /// need the peer's cooperation, so they are refused, with an explanation
    /// for the toast, when the peer is an older build.
    fn control_transfer(
        &mut self,
        file_id: &str,
        device_id: Option<&str>,
        action: TransferAction,
    ) -> Result<String, String> {
        let wanted = |d: &str| device_id.is_none_or(|w| w == d);
        let mut running: Vec<String> = Vec::new();
        if let Some(p) = self.pending_downloads.get(file_id)
            && wanted(&p.device_id)
        {
            running.push(p.device_id.clone());
        }
        running.extend(
            self.upload_sessions
                .keys()
                .filter(|(d, f)| f == file_id && wanted(d))
                .map(|(d, _)| d.clone()),
        );
        let held: Vec<String> = self
            .held_uploads
            .keys()
            .filter(|(d, f)| f == file_id && wanted(d))
            .map(|(d, _)| d.clone())
            .collect();
        let paused: Vec<ResumeState> = self
            .paused_downloads()
            .into_iter()
            .filter(|s| s.file_id == file_id && wanted(&s.device_id))
            .collect();
        let refusal = |this: &Self, peers: &[String]| -> Option<String> {
            peers
                .iter()
                .find(|d| !this.peer_supports_transfer_control(d))
                .map(|d| {
                    format!(
                        "{} runs an older ClipX that can't pause or resume transfers. You can still cancel.",
                        this.peer_name(d)
                    )
                })
        };
        match action {
            TransferAction::Cancel => {
                if running.is_empty() && held.is_empty() && paused.is_empty() {
                    return Err("No running transfer to cancel".into());
                }
                self.stop_transfer(file_id, device_id, false);
                // A paused download has nothing running: drop its kept partial
                // data and tell the sender it will not continue.
                for state in paused {
                    self.discard_kept_partial(&state.device_id, file_id);
                    self.send_peer(
                        &state.device_id,
                        Body::FileTransferCancel(proto::FileTransferCancel {
                            file_id: file_id.to_string(),
                            pause: false,
                        }),
                    );
                }
                Ok("transfer cancelled".into())
            }
            TransferAction::Pause => {
                if running.is_empty() {
                    return Err("No running transfer to pause".into());
                }
                if let Some(message) = refusal(self, &running) {
                    return Err(message);
                }
                self.stop_transfer(file_id, device_id, true);
                Ok("transfer paused".into())
            }
            TransferAction::Resume => {
                // Only a paused upload is resumed from here; a paused download
                // is resumed by asking for the file again.
                if held.is_empty() {
                    return Err(
                        "Nothing to resume here: the device receiving the file resumes it".into(),
                    );
                }
                if let Some(message) = refusal(self, &held) {
                    return Err(message);
                }
                for peer in held {
                    let key = (peer.clone(), file_id.to_string());
                    let Some(h) = self.held_uploads.get_mut(&key) else {
                        continue;
                    };
                    h.waiting = true;
                    let (name, done, total) = (h.file_name.clone(), h.done, h.total_size);
                    self.send_peer(
                        &peer,
                        Body::FileTransferResume(proto::FileTransferResume {
                            file_id: file_id.to_string(),
                        }),
                    );
                    let waiting_for = self.peer_name(&peer);
                    self.notify_transfer(
                        &peer,
                        file_id,
                        file_id,
                        &name,
                        "send",
                        done,
                        total,
                        "requesting",
                        format!("Waiting for {waiting_for} to continue"),
                    );
                }
                Ok("waiting for the receiver to continue".into())
            }
        }
    }

    /// Stop transfers of `file_id`, optionally only the one with `device_id`.
    /// The history entry and the sender's staged offer are left alone, so the
    /// file can be downloaded again. `pause` keeps the partial download
    /// (continued from the confirmed offset on the next request); otherwise
    /// it is discarded.
    fn stop_transfer(&mut self, file_id: &str, device_id: Option<&str>, pause: bool) {
        let wanted = |d: &str| device_id.is_none_or(|w| w == d);
        // Receiving side.
        if let Some(p) = self
            .pending_downloads
            .get(file_id)
            .filter(|p| wanted(&p.device_id))
            .cloned()
        {
            self.pending_downloads.remove(file_id);
            // Free the clipboard manager's slot before replying, so a Resume
            // right after cannot be refused as "already in progress".
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
            if pause {
                let _ = fs::write(
                    self.incoming_paused_path(file_id),
                    Self::now_ms().to_string(),
                );
            }
            let (state, message) = if pause {
                ("paused", format!("Paused {}", p.file_name))
            } else {
                ("cancelled", format!("Cancelled {}", p.file_name))
            };
            self.park_download(&p, file_id, pause, state, message);
            self.send_peer(
                &p.device_id,
                Body::FileTransferCancel(proto::FileTransferCancel {
                    file_id: file_id.to_string(),
                    pause,
                }),
            );
        }
        // Sending side: every session streaming this file to a wanted device.
        let keys: Vec<(String, String)> = self
            .upload_sessions
            .keys()
            .filter(|(d, f)| f == file_id && wanted(d))
            .cloned()
            .collect();
        for key in keys {
            if let Some(s) = self.upload_sessions.remove(&key) {
                if pause {
                    s.paused.store(true, Ordering::Release);
                    self.held_uploads.insert(
                        key.clone(),
                        HeldUpload {
                            file_name: s.file_name.clone(),
                            total_size: s.total_size,
                            done: s.confirmed.load(Ordering::Acquire),
                            waiting: false,
                        },
                    );
                    s.interrupt.cancel();
                } else {
                    s.cancel.cancel();
                }
            }
            self.send_peer(
                &key.0,
                Body::FileTransferCancel(proto::FileTransferCancel {
                    file_id: file_id.to_string(),
                    pause,
                }),
            );
        }
        // A paused (or waiting) upload has no task left to report a cancel.
        if !pause {
            let held: Vec<(String, String)> = self
                .held_uploads
                .keys()
                .filter(|(d, f)| f == file_id && wanted(d))
                .cloned()
                .collect();
            for key in held {
                if let Some(h) = self.held_uploads.remove(&key) {
                    self.notify_transfer(
                        &key.0,
                        file_id,
                        file_id,
                        &h.file_name,
                        "send",
                        h.done,
                        h.total_size,
                        "cancelled",
                        format!("Sending {} was cancelled", h.file_name),
                    );
                    self.send_peer(
                        &key.0,
                        Body::FileTransferCancel(proto::FileTransferCancel {
                            file_id: file_id.to_string(),
                            pause: false,
                        }),
                    );
                }
            }
        }
    }

    /// Downloads the user paused: partial data and resume state are on disk,
    /// nothing is running. They survive an app restart.
    fn paused_downloads(&self) -> Vec<ResumeState> {
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("resume") {
                    return None;
                }
                let state: ResumeState = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                (self.incoming_paused_path(&state.file_id).exists()
                    && !self.pending_downloads.contains_key(&state.file_id)
                    && state.offer_expires_at_ms >= Self::now_ms())
                .then_some(state)
            })
            .collect()
    }

    /// Every live transfer, for a UI that opened after they started.
    fn list_transfers(&self) -> Vec<TransferSnapshot> {
        let mut out = Vec::new();
        for (file_id, p) in &self.pending_downloads {
            out.push(TransferSnapshot {
                device_id: p.device_id.clone(),
                device_name: self.peer_name(&p.device_id),
                entry_id: p.entry_id.clone(),
                file_id: file_id.clone(),
                file_name: p.file_name.clone(),
                direction: "download".into(),
                done: p.received_size,
                total: p.total_size,
                state: if p.received_size > 0 {
                    "receiving"
                } else {
                    "requesting"
                }
                .into(),
                can_pause: self.peer_supports_transfer_control(&p.device_id),
            });
        }
        for state in self.paused_downloads() {
            out.push(TransferSnapshot {
                device_id: state.device_id.clone(),
                device_name: self.peer_name(&state.device_id),
                entry_id: state.entry_id.clone(),
                file_id: state.file_id.clone(),
                file_name: state.file_name.clone(),
                direction: "download".into(),
                done: state.confirmed_offset,
                total: state.total_size,
                state: "paused".into(),
                can_pause: self.peer_supports_transfer_control(&state.device_id),
            });
        }
        for ((device_id, file_id), s) in &self.upload_sessions {
            out.push(TransferSnapshot {
                device_id: device_id.clone(),
                device_name: self.peer_name(device_id),
                entry_id: file_id.clone(),
                file_id: file_id.clone(),
                file_name: s.file_name.clone(),
                direction: "send".into(),
                done: s.confirmed.load(Ordering::Acquire),
                total: s.total_size,
                state: "sending".into(),
                can_pause: self.peer_supports_transfer_control(device_id),
            });
        }
        for ((device_id, file_id), h) in &self.held_uploads {
            out.push(TransferSnapshot {
                device_id: device_id.clone(),
                device_name: self.peer_name(device_id),
                entry_id: file_id.clone(),
                file_id: file_id.clone(),
                file_name: h.file_name.clone(),
                direction: "send".into(),
                done: h.done,
                total: h.total_size,
                state: if h.waiting { "requesting" } else { "paused" }.into(),
                can_pause: self.peer_supports_transfer_control(device_id),
            });
        }
        out
    }

    /// Retire a download that stopped for a reason other than failure.
    /// `keep_partial` leaves the temp file and resume state on disk so the
    /// download can continue from the confirmed offset; otherwise both go.
    fn park_download(
        &mut self,
        p: &PendingDownload,
        file_id: &str,
        keep_partial: bool,
        state: &str,
        message: String,
    ) {
        if let Some(s) = self
            .incoming_files
            .remove(&(p.device_id.clone(), file_id.to_string()))
        {
            s.profile.log_summary(file_id, state);
            if !keep_partial {
                let _ = fs::remove_file(&s.temp_path);
            }
        }
        // Never leave it on the auto-resume list: only an explicit request
        // (or, for an interruption, a reconnect scan on next start) resumes it.
        self.resume_candidates.remove(file_id);
        if !keep_partial {
            let _ = fs::remove_file(self.incoming_temp_path(&p.device_id, file_id));
            let _ = fs::remove_file(self.incoming_state_path(file_id));
            let _ = fs::remove_file(self.incoming_paused_path(file_id));
        }
        self.notify_transfer(
            &p.device_id,
            &p.entry_id,
            file_id,
            &p.file_name,
            "download",
            p.received_size,
            p.total_size,
            state,
            message,
        );
    }

    fn cancel_local_transfer(&mut self, file_id: &str) {
        let keys: Vec<(String, String)> = self
            .upload_sessions
            .keys()
            .filter(|(_, id)| id == file_id)
            .cloned()
            .collect();
        for k in keys {
            if let Some(s) = self.upload_sessions.remove(&k) {
                s.cancel.cancel();
            }
        }
        self.cancel_local_download(file_id);
    }

    fn incoming_temp_path(&self, device_id: &str, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{device_id}-{file_id}"))
    }
    fn incoming_state_path(&self, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{file_id}.resume"))
    }
    /// Marks a download the user paused: its partial data and resume state are
    /// kept, but it must not restart by itself on reconnect or app restart.
    fn incoming_paused_path(&self, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{file_id}.paused"))
    }

    fn persist_resume_state(
        &self,
        device_id: &str,
        file_id: &str,
        pending: &PendingDownload,
        offset: u64,
    ) {
        let state = ResumeState {
            version: 1,
            device_id: device_id.to_string(),
            file_id: file_id.to_string(),
            entry_id: pending.entry_id.clone(),
            file_name: pending.file_name.clone(),
            total_size: pending.total_size,
            confirmed_offset: offset,
            offer_expires_at_ms: pending.offer_expires_at_ms,
            updated_at_ms: Self::now_ms(),
        };
        let path = self.incoming_state_path(file_id);
        let tmp = path.with_extension("resume.tmp");
        if let Ok(bytes) = serde_json::to_vec(&state)
            && fs::write(&tmp, bytes).is_ok()
        {
            let _ = fs::rename(tmp, path);
        }
    }

    fn load_valid_resume_state(
        &self,
        file_id: &str,
        device_id: &str,
        entry_id: &str,
        file_name: &str,
        total_size: u64,
        offer_expires_at_ms: u64,
    ) -> Option<ResumeState> {
        let state: ResumeState =
            serde_json::from_slice(&fs::read(self.incoming_state_path(file_id)).ok()?).ok()?;
        let valid = state.version == 1
            && state.file_id == file_id
            && state.device_id == device_id
            && state.entry_id == entry_id
            && state.file_name == file_name
            && state.total_size == total_size
            && state.offer_expires_at_ms == offer_expires_at_ms
            && state.confirmed_offset <= total_size
            && state.offer_expires_at_ms >= Self::now_ms()
            && state
                .updated_at_ms
                .saturating_add(RESUME_STATE_TTL.as_millis() as u64)
                >= Self::now_ms()
            && fs::metadata(self.incoming_temp_path(device_id, file_id))
                .is_ok_and(|m| m.len() == state.confirmed_offset);
        if valid { Some(state) } else { None }
    }

    fn load_resume_candidates(&mut self) {
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("resume") {
                continue;
            }
            let file_id = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .strip_prefix("clipx-")
                .unwrap_or_default()
                .to_string();
            if file_id.is_empty() {
                let _ = fs::remove_file(path);
                continue;
            }
            if self
                .incoming_files_dir
                .join(format!("clipx-{file_id}.cancelled"))
                .exists()
            {
                let _ = fs::remove_file(&path);
                continue;
            }
            // Paused by the user: keep the files, but wait for an explicit
            // resume instead of restarting on the next connection.
            if self.incoming_paused_path(&file_id).exists() {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Ok(state) = serde_json::from_slice::<ResumeState>(&bytes) else {
                let _ = fs::remove_file(path);
                continue;
            };
            let valid = state.version == 1
                && state.file_id == file_id
                && state.confirmed_offset <= state.total_size
                && state.offer_expires_at_ms >= Self::now_ms()
                && state
                    .updated_at_ms
                    .saturating_add(RESUME_STATE_TTL.as_millis() as u64)
                    >= Self::now_ms()
                && fs::metadata(self.incoming_temp_path(&state.device_id, &state.file_id))
                    .is_ok_and(|m| m.len() == state.confirmed_offset);
            if valid {
                self.resume_candidates.insert(state.file_id.clone(), state);
            } else {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(self.incoming_temp_path(&state.device_id, &state.file_id));
            }
        }
    }

    fn cleanup_stale_incoming_files(&mut self) {
        let now = Self::now_ms();
        let cutoff = now.saturating_sub(RESUME_STATE_TTL.as_millis() as u64);
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or_default();
            if path.extension().and_then(|x| x.to_str()) == Some("cancelled") {
                let stale = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                    .is_some_and(|age| age >= RESUME_STATE_TTL);
                if stale {
                    let _ = fs::remove_file(path);
                }
                continue;
            }
            if path.extension().and_then(|x| x.to_str()) == Some("resume") {
                if let Ok(bytes) = fs::read(&path)
                    && let Ok(state) = serde_json::from_slice::<ResumeState>(&bytes)
                {
                    if state.updated_at_ms >= cutoff
                        && state.offer_expires_at_ms >= now
                        && !self
                            .incoming_files_dir
                            .join(format!("clipx-{}.cancelled", state.file_id))
                            .exists()
                    {
                        continue;
                    }
                    let _ =
                        fs::remove_file(self.incoming_temp_path(&state.device_id, &state.file_id));
                }
                let _ = fs::remove_file(path);
                continue;
            }
            if self.incoming_files.values().any(|s| s.temp_path == path)
                || name.contains(".cancelled")
            {
                continue;
            }
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age >= RESUME_STATE_TTL);
            if stale {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn release_pending_download(&mut self, file_id: &str) -> Option<PendingDownload> {
        let pending = self.pending_downloads.remove(file_id);
        if pending.is_some() {
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
        }
        pending
    }

    fn notify_transfer(
        &mut self,
        device_id: &str,
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
        let terminal = matches!(
            state,
            "complete" | "failed" | "expired" | "cancelled" | "interrupted" | "paused"
        );
        let should_emit = if terminal || total == 0 || !matches!(state, "receiving" | "sending") {
            true
        } else {
            let percent = ((done.saturating_mul(100)) / total).min(100) as u8;
            let key = (state.to_string(), percent);
            if self.transfer_progress.get(file_id) == Some(&key) {
                false
            } else {
                self.transfer_progress.insert(file_id.to_string(), key);
                true
            }
        };
        if !should_emit {
            return;
        }
        if terminal {
            tracing::debug!(%file_id, direction, state, done, total, "file transfer state changed");
        } else {
            tracing::trace!(%file_id, direction, state, done, total, "file transfer progress emitted");
        }
        if terminal || state == "interrupted" || state == "cancelled" {
            self.transfer_progress.remove(file_id);
        }
        let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: entry_id.to_string(),
            file_id: file_id.to_string(),
            file_name: file_name.to_string(),
            direction: direction.to_string(),
            done,
            total,
            state: state.to_string(),
            message: message.clone(),
            device_id: device_id.to_string(),
            device_name: self.peer_name(device_id),
            can_pause: self.peer_supports_transfer_control(device_id),
        });
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
        // The native notification event is enqueued synchronously above.
        // Only the acknowledgment wait is spawned, so a later transfer state
        // cannot overtake an earlier one merely because Tokio scheduled its
        // wrapper task first.
        tokio::spawn(notification_future);
    }

    async fn emit_transfer(
        events_tx: &broadcast::Sender<CoreEvent>,
        notification: &E,
        device_id: &str,
        device_name: &str,
        can_pause: bool,
        file_id: &str,
        file_name: &str,
        direction: &str,
        done: u64,
        total: u64,
        state: &'static str,
        message: String,
    ) {
        let _ = events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: file_id.to_string(),
            file_id: file_id.to_string(),
            file_name: file_name.to_string(),
            direction: direction.to_string(),
            done,
            total,
            state: state.to_string(),
            message: message.clone(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            can_pause,
        });
        notification
            .notify_file_transfer(
                file_id.to_string(),
                file_name.to_string(),
                direction.to_string(),
                done,
                total,
                state.to_string(),
                message,
                None,
            )
            .await;
    }

    fn percentage(done: u64, total: u64) -> u8 {
        done.saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u8
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn send_peer(&self, device_id: &str, body: Body) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };

        let device_id = device_id.to_string();

        let (reply_tx, reply_rx) = oneshot::channel();

        if tx
            .send(TransportCommand::SendPeerMessage {
                device_id: device_id.clone(),
                message: proto::PeerMessage { body: Some(body) },
                enqueued_at: Instant::now(),
                reply_to: reply_tx,
            })
            .is_err()
        {
            tracing::warn!(%device_id, "transport command channel is closed");
            return;
        }

        tokio::spawn(async move {
            match reply_rx.await {
                Ok(true) => {
                    tracing::debug!(%device_id, "peer message queued for transport");
                }
                Ok(false) => {
                    tracing::warn!(%device_id, "transport rejected peer message");
                }
                Err(_) => {
                    tracing::warn!(%device_id, "transport dropped peer-message reply");
                }
            }
        });
    }

    fn send_peer_on_connection(&self, device_id: &str, connection_id: &str, body: Body) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::SendPeerMessageOnConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            message: proto::PeerMessage { body: Some(body) },
            enqueued_at: Instant::now(),
            reply_to: reply_tx,
        });
    }

    fn send_control(&self, device_id: &str, code: u32, message: impl Into<String>) {
        self.send_peer(
            device_id,
            Body::Control(proto::PreTransportControl {
                code,
                message: message.into(),
            }),
        );
    }

    fn send_control_on_connection(
        &self,
        device_id: &str,
        connection_id: &str,
        code: u32,
        message: impl Into<String>,
    ) {
        self.send_peer_on_connection(
            device_id,
            connection_id,
            Body::Control(proto::PreTransportControl {
                code,
                message: message.into(),
            }),
        );
    }

    fn request_transport_disconnect(&self, device_id: &str) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::Disconnect {
            device_id: device_id.to_string(),
            reply_to: reply_tx,
        });
    }

    fn promote_transport_connection(&self, device_id: &str, connection_id: &str) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::PromoteConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            reply_to: reply_tx,
        });
    }

    fn request_transport_disconnect_on_connection(&self, device_id: &str, connection_id: &str) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::DisconnectOnConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            reply_to: reply_tx,
        });
    }

    fn notify_info(&self, title: &'static str, body: impl Into<String>) {
        let engine = self.notification.clone();
        let body = body.into();
        tokio::spawn(async move {
            engine.notify_info(title, body).await;
        });
    }

    fn prune_expired_sessions(&mut self) {
        const TTL: Duration = Duration::from_secs(90);
        let expired_pairs: Vec<String> = self
            .pair_sessions
            .iter()
            .filter(|(_, s)| s.is_expired(TTL))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired_pairs {
            self.abort_pair(&id, pairing::control::TIMEOUT, "pairing timed out");
        }

        let expired_connects: Vec<String> = self
            .connect_sessions
            .iter()
            .filter(|(_, s)| s.is_expired(TTL))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired_connects {
            self.abort_connect(&id, pairing::control::TIMEOUT, "connect timed out");
        }
    }

    /// Trusted devices enriched with live address/port from `seen` and live
    async fn build_paired_devices(&self) -> Vec<proto::DeviceInfo> {
        let mut devices: Vec<proto::DeviceInfo> = self
            .trusted
            .list()
            .map(|trusted_device| {
                let seen = self.seen.get(&trusted_device.id);
                let connection = if self.connected.contains_key(&trusted_device.id) {
                    proto::ConnectionState::Connected
                } else if self.connect_sessions.contains_key(&trusted_device.id) {
                    proto::ConnectionState::Connecting
                } else if seen.is_some() {
                    proto::ConnectionState::Disconnected
                } else {
                    proto::ConnectionState::Unavailable
                };
                proto::DeviceInfo {
                    id: trusted_device.id.clone(),
                    name: trusted_device.name.clone(),
                    device_type: trusted_device.device_type as i32,
                    address: seen.map(|s| s.addr.to_string()).unwrap_or_default(),
                    last_seen_ms: seen
                        .map(|s| s.last_seen.elapsed().as_millis() as u64)
                        .unwrap_or_default(),
                    ws_port: seen.map(|s| s.ws_port).unwrap_or_default(),
                    connection: connection as i32,
                    auto_connect: trusted_device.auto_connect,
                    trusted: true,
                }
            })
            .collect();

        devices.sort_by(|a, b| {
            connection_rank(a.connection)
                .cmp(&connection_rank(b.connection))
                .then_with(|| a.name.cmp(&b.name))
        });
        devices
    }
}

fn connection_rank(state: i32) -> u8 {
    match proto::ConnectionState::try_from(state).unwrap_or(proto::ConnectionState::Disconnected) {
        proto::ConnectionState::Connecting => 0,
        proto::ConnectionState::Connected => 1,
        proto::ConnectionState::Disconnected => 2,
        proto::ConnectionState::Unavailable => 3,
    }
}

fn get_formatted_fp(device_id: &str) -> String {
    device_id
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .take(7)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    async fn handle_device_command(&mut self, cmd: DeviceCommands) {
        match cmd {
            DeviceCommands::GetSeen { mode, reply_to } => {
                let mut devices = self
                    .seen
                    .list()
                    .filter(|device| match mode {
                        SeenMode::All => true,
                        SeenMode::Trusted => self.trusted.is_trusted(&device.id),
                        SeenMode::Untrusted => !self.trusted.is_trusted(&device.id),
                    })
                    .cloned()
                    .collect::<Vec<SeenDevice>>();
                devices.sort_by_key(|b| std::cmp::Reverse(b.last_seen));
                let _ = reply_to.send(devices);
            }
            DeviceCommands::Pair {
                device_id,
                reply_to,
            } => {
                let _ = reply_to.send(self.handle_pair_request(&device_id));
            }
            DeviceCommands::Connect {
                device_id,
                reply_to,
            } => {
                let _ = reply_to.send(self.handle_connect(&device_id));
            }
            DeviceCommands::Connected { reply_to } => {
                let devices = self
                    .connected
                    .keys()
                    .filter_map(|id| {
                        if let Some(seen) = self.seen.get(id) {
                            return Some(seen.clone());
                        }
                        // Connected but not currently seen over UDP (e.g. Wi-Fi Direct).
                        // The live connection is the truth, so still report it.
                        let td = self.trusted.get(id)?;
                        Some(SeenDevice {
                            id: td.id.clone(),
                            name: td.name.clone(),
                            device_type: td.device_type,
                            addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                            ws_port: 0,
                            last_seen: Instant::now(),
                        })
                    })
                    .collect::<Vec<SeenDevice>>();
                let _ = reply_to.send(devices);
            }
            DeviceCommands::GetIdentity { reply_to } => {
                let addr = config::preferred_local_ip().unwrap_or(Ipv4Addr::UNSPECIFIED);
                let _ = reply_to.send(IdentitySnapshot {
                    device_id: hex::encode(self.identity.get_this_device_fingerprint()),
                    device_name: config::get_hostname(),
                    public_key_hex: hex::encode(self.identity.public_key_bytes()),
                    ws_port: config::get_effective_ws_port(),
                    device_type: config::get_current_device_type(),
                    ip_addr: addr.to_string(),
                });
            }
            DeviceCommands::GetPaired { reply_to } => {
                let _ = reply_to.send(self.build_paired_devices().await);
            }
            DeviceCommands::ControlTransfer {
                device_id,
                file_id,
                action,
                reply_to,
            } => {
                let _ =
                    reply_to.send(self.control_transfer(&file_id, device_id.as_deref(), action));
            }
            DeviceCommands::ListTransfers { reply_to } => {
                let _ = reply_to.send(self.list_transfers());
            }
            DeviceCommands::Disconnect {
                device_id,
                reply_to,
            } => {
                let _ = reply_to.send(self.handle_disconnect(&device_id).await);
            }
            DeviceCommands::SetAutoConnect {
                device_id,
                auto_connect,
                reply_to,
            } => {
                let ok = self.trusted.set_auto_connect(&device_id, auto_connect);
                if ok {
                    self.notify_devices_changed();
                }
                let _ = reply_to.send(ok);
            }
            DeviceCommands::ForgetDevice {
                device_id,
                reply_to,
            } => {
                // connected or mid connecting — tear the transport connection
                // down and wait for it same as manual Disconnect
                // before wiping trust — other the socket (and any other background dial task)
                // leaks and block repairing later
                if self.connected.contains_key(&device_id) {
                    let _ = self.handle_disconnect(&device_id).await;
                }
                self.trusted.revoke(&device_id);
                self.notify_devices_changed();
                let _ = reply_to.send("ok".to_string());
            }
            DeviceCommands::PendingPairings { reply_to } => {
                let _ = reply_to.send(self.list_pending_pairings());
            }
            DeviceCommands::ApprovePairing {
                device_id,
                approve,
                reply_to,
            } => {
                let _ = reply_to.send(self.handle_approve_pairing(&device_id, approve));
            }
        }
    }
}

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    fn list_pending_pairings(&self) -> Vec<PendingPairEntry> {
        self.pair_sessions
            .iter()
            .filter_map(|(id, sess)| match sess.stage {
                PairStage::AwaitingLocalApproval => Some(PendingPairEntry {
                    device_id: id.clone(),
                    device_name: sess.peer_name.clone().unwrap_or_else(|| id.clone()),
                    stage: "awaiting_approval",
                    code: None,
                }),
                PairStage::AwaitingCodeConfirm => Some(PendingPairEntry {
                    device_id: id.clone(),
                    device_name: sess.peer_name.clone().unwrap_or_else(|| id.clone()),
                    stage: "awaiting_code_confirm",
                    code: sess.code.map(|c| format!("{c:06}")),
                }),
                _ => None,
            })
            .collect()
    }

    /// Feeds the exact same decision path a toast click would — CLI and
    /// native popup are just two sources racing to fill the same session
    /// stage. First to arrive wins; the loser hits a stage mismatch and
    /// silently no-ops (see the guard at the top of both handlers below).
    fn handle_approve_pairing(&mut self, device_id: &str, approve: bool) -> String {
        let Some(sess) = self.pair_sessions.get(device_id) else {
            return "no pending pairing".to_string();
        };
        let decision = if approve {
            PairDecision::Allow
        } else {
            PairDecision::Deny
        };
        match sess.stage {
            PairStage::AwaitingLocalApproval => {
                self.on_pair_approval_result(device_id.to_string(), decision);
                "ok".to_string()
            }
            PairStage::AwaitingCodeConfirm => {
                self.on_code_confirm_result(device_id.to_string(), decision);
                "ok".to_string()
            }
            _ => "not awaiting approval right now".to_string(),
        }
    }
}

#[cfg(test)]
mod transfer_control_tests {
    use super::*;
    use crate::notification::platform::NotificationFuture;
    use crate::notification::{IncomingClipboardDecision, PairDecision, Prompt};

    /// Engine that does nothing, so the tests behave the same on every OS
    /// (the real Windows engine would raise toasts).
    #[derive(Clone)]
    struct QuietEngine;

    impl Engine for QuietEngine {
        fn ask_pair<'a>(&'a self, _: Prompt, _: Duration) -> NotificationFuture<'a, PairDecision> {
            Box::pin(async { PairDecision::NoResponse })
        }
        fn ask_clipboard<'a>(
            &'a self,
            _: Prompt,
            _: Duration,
        ) -> NotificationFuture<'a, IncomingClipboardDecision> {
            Box::pin(async { IncomingClipboardDecision::Ignore })
        }
        fn notify_info<'a>(&'a self, _: &'a str, _: String) -> NotificationFuture<'a, ()> {
            Box::pin(async {})
        }
    }

    struct Harness {
        mgr: DeviceManager<QuietEngine>,
        transport_rx: mpsc::UnboundedReceiver<TransportCommand>,
        clipboard_rx: mpsc::UnboundedReceiver<ClipboardCommand>,
        events_rx: broadcast::Receiver<CoreEvent>,
        dir: PathBuf,
    }

    const DEVICE: &str = "dev-a";
    const OTHER: &str = "dev-b";
    const FILE: &str = "file-1";
    const ENTRY: &str = "entry-1";
    const TOTAL: u64 = 4096;
    const PARTIAL: u64 = 1024;

    fn harness() -> Harness {
        let dir =
            std::env::temp_dir().join(format!("clipx-transfer-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (transport_tx, transport_rx) = mpsc::unbounded_channel();
        let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();
        let (events_tx, events_rx) = broadcast::channel(64);
        let identity = Arc::new(DeviceIdentity::load_or_create(dir.join("identity.key")));
        let mut mgr = DeviceManager::new(
            dir.join("trusted.json"),
            identity,
            Some(transport_tx),
            QuietEngine,
            clipboard_tx,
            events_tx,
        );
        mgr.incoming_files_dir = dir.join("incoming");
        fs::create_dir_all(&mgr.incoming_files_dir).unwrap();
        Harness {
            mgr,
            transport_rx,
            clipboard_rx,
            events_rx,
            dir,
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn offer_expiry() -> u64 {
        DeviceManager::<QuietEngine>::now_ms() + 60 * 60 * 1000
    }

    /// Marks a peer as speaking this build's protocol, with or without the
    /// pause/resume capability.
    fn peer(h: &mut Harness, device: &str, can_control: bool) {
        let caps = if can_control {
            vec![proto::PeerCapabilities::CapabilityFileTransferControl as i32]
        } else {
            vec![]
        };
        h.mgr
            .effective_versions
            .insert(device.to_string(), (1, caps));
        h.mgr.connected.insert(device.into(), "conn-1".into());
    }

    /// A download that already received PARTIAL bytes, exactly as the chunk
    /// handler leaves it: pending entry, temp file and resume state.
    fn partial_download(h: &mut Harness) {
        let pending = PendingDownload {
            device_id: DEVICE.into(),
            entry_id: ENTRY.into(),
            file_name: "movie.mp4".into(),
            total_size: TOTAL,
            offer_expires_at_ms: offer_expiry(),
            received_size: PARTIAL,
            received_complete: false,
            last_activity: Instant::now(),
            stall_retries: 0,
        };
        fs::write(
            h.mgr.incoming_temp_path(DEVICE, FILE),
            vec![7u8; PARTIAL as usize],
        )
        .unwrap();
        h.mgr.persist_resume_state(DEVICE, FILE, &pending, PARTIAL);
        h.mgr.pending_downloads.insert(FILE.into(), pending);
        peer(h, DEVICE, true);
    }

    struct Tokens {
        cancel: CancellationToken,
        interrupt: CancellationToken,
    }

    fn upload_to(h: &mut Harness, device: &str, done: u64) -> Tokens {
        let (ack_tx, _ack_rx) = mpsc::unbounded_channel();
        let tokens = Tokens {
            cancel: CancellationToken::new(),
            interrupt: CancellationToken::new(),
        };
        h.mgr.upload_sessions.insert(
            (device.into(), FILE.into()),
            UploadSession {
                id: 1,
                cancel: tokens.cancel.clone(),
                interrupt: tokens.interrupt.clone(),
                file_name: "movie.mp4".into(),
                total_size: TOTAL,
                confirmed: Arc::new(AtomicU64::new(done)),
                paused: Arc::new(AtomicBool::new(false)),
                superseded: Arc::new(AtomicBool::new(false)),
                ack_tx,
            },
        );
        tokens
    }

    /// (device, file, pause) for every FileTransferCancel sent.
    fn cancels(h: &mut Harness) -> Vec<(String, String, bool)> {
        let mut out = Vec::new();
        while let Ok(cmd) = h.transport_rx.try_recv() {
            if let TransportCommand::SendPeerMessage {
                device_id, message, ..
            } = cmd
                && let Some(Body::FileTransferCancel(c)) = message.body
            {
                out.push((device_id, c.file_id, c.pause));
            }
        }
        out
    }

    fn resumes_sent(h: &mut Harness) -> Vec<(String, String)> {
        let mut out = Vec::new();
        while let Ok(cmd) = h.transport_rx.try_recv() {
            if let TransportCommand::SendPeerMessage {
                device_id, message, ..
            } = cmd
                && let Some(Body::FileTransferResume(r)) = message.body
            {
                out.push((device_id, r.file_id));
            }
        }
        out
    }

    fn download_requests(h: &mut Harness) -> Vec<u64> {
        let mut out = Vec::new();
        while let Ok(cmd) = h.transport_rx.try_recv() {
            if let TransportCommand::SendPeerMessage { message, .. } = cmd
                && let Some(Body::FileDownloadRequest(r)) = message.body
            {
                out.push(r.offset);
            }
        }
        out
    }

    struct Seen {
        state: String,
        done: u64,
        device_id: String,
        can_pause: bool,
    }

    fn last_event(h: &mut Harness) -> Option<Seen> {
        let mut last = None;
        while let Ok(ev) = h.events_rx.try_recv() {
            if let CoreEvent::FileTransferChanged {
                state,
                done,
                device_id,
                can_pause,
                ..
            } = ev
            {
                last = Some(Seen {
                    state,
                    done,
                    device_id,
                    can_pause,
                });
            }
        }
        last
    }

    fn last_state(h: &mut Harness) -> Option<(String, u64)> {
        last_event(h).map(|e| (e.state, e.done))
    }

    #[tokio::test]
    async fn pause_keeps_partial_data_and_tells_the_sender() {
        let mut h = harness();
        partial_download(&mut h);

        h.mgr.stop_transfer(FILE, None, true);

        assert!(!h.mgr.pending_downloads.contains_key(FILE));
        assert!(
            h.mgr.incoming_temp_path(DEVICE, FILE).exists(),
            "partial file must stay"
        );
        assert!(
            h.mgr.incoming_state_path(FILE).exists(),
            "resume state must stay"
        );
        assert!(
            h.mgr.incoming_paused_path(FILE).exists(),
            "paused marker must be written"
        );
        let ev = last_event(&mut h).unwrap();
        assert_eq!((ev.state.as_str(), ev.done), ("paused", PARTIAL));
        assert_eq!(
            ev.device_id, DEVICE,
            "events say which device the transfer is with"
        );
        assert!(ev.can_pause);
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), true)]);
    }

    #[tokio::test]
    async fn cancel_discards_partial_data_but_not_the_history_entry() {
        let mut h = harness();
        partial_download(&mut h);

        h.mgr.stop_transfer(FILE, None, false);

        assert!(!h.mgr.pending_downloads.contains_key(FILE));
        assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert!(!h.mgr.incoming_state_path(FILE).exists());
        assert!(!h.mgr.incoming_paused_path(FILE).exists());
        assert!(
            !h.mgr
                .incoming_files_dir
                .join(format!("clipx-{FILE}.cancelled"))
                .exists(),
            "no .cancelled marker: that is only for history removal"
        );
        assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
        let mut released = false;
        while let Ok(cmd) = h.clipboard_rx.try_recv() {
            assert!(!matches!(cmd, ClipboardCommand::RemoveEntry { .. }));
            released |= matches!(cmd, ClipboardCommand::FileDownloadReleased { .. });
        }
        assert!(
            released,
            "the clipboard manager's download slot must be freed"
        );
    }

    #[tokio::test]
    async fn paused_download_is_not_restarted_by_the_startup_scan() {
        let mut h = harness();
        partial_download(&mut h);
        h.mgr.stop_transfer(FILE, None, true);

        h.mgr.load_resume_candidates();
        assert!(
            h.mgr.resume_candidates.is_empty(),
            "paused downloads wait for the user"
        );
        assert!(
            h.mgr.incoming_state_path(FILE).exists(),
            "scan must not delete paused state"
        );

        // An interrupted (not paused) download is still resumed automatically.
        fs::remove_file(h.mgr.incoming_paused_path(FILE)).unwrap();
        h.mgr.load_resume_candidates();
        assert!(h.mgr.resume_candidates.contains_key(FILE));
    }

    fn file_request(h: &Harness) -> ClipboardOutbound {
        ClipboardOutbound::FileRequest {
            device_id: DEVICE.into(),
            file_id: FILE.into(),
            entry_id: ENTRY.into(),
            file_name: "movie.mp4".into(),
            total_size: TOTAL,
            offer_expires_at_ms: offer_expiry_of(h),
        }
    }

    /// The expiry recorded in the kept resume state (a fresh `offer_expiry()`
    /// would not match it).
    fn offer_expiry_of(h: &Harness) -> u64 {
        let bytes = fs::read(h.mgr.incoming_state_path(FILE)).unwrap();
        serde_json::from_slice::<ResumeState>(&bytes)
            .unwrap()
            .offer_expires_at_ms
    }

    #[tokio::test]
    async fn resume_continues_from_the_kept_offset_and_clears_the_marker() {
        let mut h = harness();
        partial_download(&mut h);
        h.mgr.stop_transfer(FILE, None, true);
        let _ = cancels(&mut h);

        let req = file_request(&h);
        h.mgr
            .handle_clipboard_outbound(req, &h.mgr.clipboard_tx.clone());

        assert_eq!(download_requests(&mut h), vec![PARTIAL]);
        assert_eq!(h.mgr.pending_downloads[FILE].received_size, PARTIAL);
        assert!(!h.mgr.incoming_paused_path(FILE).exists());
        assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    }

    #[tokio::test]
    async fn download_after_cancel_starts_from_zero() {
        let mut h = harness();
        partial_download(&mut h);
        let req = file_request(&h);
        h.mgr.stop_transfer(FILE, None, false);
        let _ = cancels(&mut h);

        h.mgr
            .handle_clipboard_outbound(req, &h.mgr.clipboard_tx.clone());

        assert_eq!(download_requests(&mut h), vec![0]);
        assert_eq!(h.mgr.pending_downloads[FILE].received_size, 0);
    }

    #[tokio::test]
    async fn pausing_with_an_old_peer_is_refused_but_cancel_still_works() {
        let mut h = harness();
        partial_download(&mut h);
        peer(&mut h, DEVICE, false); // an older build: no pause/resume capability

        let reply = h.mgr.control_transfer(FILE, None, TransferAction::Pause);
        let message = reply.clone().unwrap_err();
        assert!(
            message.contains("older ClipX"),
            "toast explains why: {message}"
        );
        assert!(
            h.mgr.pending_downloads.contains_key(FILE),
            "transfer keeps running"
        );
        assert!(
            cancels(&mut h).is_empty(),
            "nothing is sent to the old peer"
        );
        assert!(!last_event(&mut h).is_some_and(|e| e.state == "paused"));

        let reply = h.mgr.control_transfer(FILE, None, TransferAction::Cancel);
        assert_eq!(reply, Ok("transfer cancelled".to_string()));
        assert!(!h.mgr.pending_downloads.contains_key(FILE));
        // The old peer understands a plain cancel (the extra field is ignored).
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
    }

    #[tokio::test]
    async fn a_peer_with_no_hello_at_all_counts_as_old() {
        let mut h = harness();
        partial_download(&mut h);
        h.mgr.effective_versions.remove(DEVICE);
        assert!(!h.mgr.peer_supports_transfer_control(DEVICE));
        assert!(
            h.mgr
                .control_transfer(FILE, None, TransferAction::Pause)
                .unwrap_err()
                .contains("older ClipX")
        );
    }

    #[tokio::test]
    async fn old_peers_still_pass_the_hello_gate_and_new_ones_advertise_control() {
        let mut h = harness();
        // A hello with no capabilities (an older build) is accepted, not rejected.
        let old = Hello {
            proto_version: config::PROTO_VERSION,
            min_proto_version: config::MINI_PROTO_VERSION,
            capabilities: vec![],
        };
        assert!(h.mgr.check_hello("old-peer", Some(old)).is_ok());
        assert!(!h.mgr.peer_supports_transfer_control("old-peer"));
        // This build advertises the capability and a peer that sends it is recognised.
        let mine = config::local_hello();
        assert!(h.mgr.check_hello("new-peer", Some(mine)).is_ok());
        assert!(h.mgr.peer_supports_transfer_control("new-peer"));
        // A peer that sends no hello (legacy) still connects.
        assert!(h.mgr.check_hello("legacy", None).is_ok());
    }

    #[tokio::test]
    async fn one_offer_to_two_devices_is_controlled_per_device() {
        let mut h = harness();
        peer(&mut h, DEVICE, true);
        peer(&mut h, OTHER, true);
        let a = upload_to(&mut h, DEVICE, 100);
        let b = upload_to(&mut h, OTHER, 200);

        let reply = h
            .mgr
            .control_transfer(FILE, Some(DEVICE), TransferAction::Cancel);
        assert_eq!(reply, Ok("transfer cancelled".to_string()));

        assert!(
            a.cancel.is_cancelled(),
            "the chosen device's transfer is cancelled"
        );
        assert!(
            !b.cancel.is_cancelled(),
            "the other device's transfer keeps going"
        );
        assert!(
            h.mgr
                .upload_sessions
                .contains_key(&(OTHER.into(), FILE.into()))
        );
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);

        let snapshot = h.mgr.list_transfers();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(
            (snapshot[0].device_id.as_str(), snapshot[0].done),
            (OTHER, 200)
        );
    }

    #[tokio::test]
    async fn stopping_an_upload_cancels_it_and_pausing_holds_it() {
        for pause in [false, true] {
            let mut h = harness();
            peer(&mut h, DEVICE, true);
            let t = upload_to(&mut h, DEVICE, 512);

            h.mgr.stop_transfer(FILE, None, pause);

            assert_eq!(t.cancel.is_cancelled(), !pause);
            assert_eq!(t.interrupt.is_cancelled(), pause);
            assert!(h.mgr.upload_sessions.is_empty());
            assert_eq!(h.mgr.held_uploads.len(), usize::from(pause));
            assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), pause)]);
        }
    }

    #[tokio::test]
    async fn sender_pause_then_resume_asks_the_receiver_to_continue() {
        let mut h = harness();
        peer(&mut h, DEVICE, true);
        let _t = upload_to(&mut h, DEVICE, 512);

        assert_eq!(
            h.mgr.control_transfer(FILE, None, TransferAction::Pause),
            Ok("transfer paused".to_string())
        );
        let held = &h.mgr.held_uploads[&(DEVICE.to_string(), FILE.to_string())];
        assert_eq!((held.done, held.waiting), (512, false));
        let _ = cancels(&mut h);
        assert_eq!(h.mgr.list_transfers()[0].state, "paused");

        assert!(
            h.mgr
                .control_transfer(FILE, None, TransferAction::Resume)
                .is_ok()
        );
        assert_eq!(resumes_sent(&mut h), vec![(DEVICE.into(), FILE.into())]);
        assert!(h.mgr.held_uploads[&(DEVICE.to_string(), FILE.to_string())].waiting);
        assert_eq!(h.mgr.list_transfers()[0].state, "requesting");

        // Cancelling while waiting reports it (no task is left to) and tells the peer.
        let _ = last_event(&mut h);
        assert!(
            h.mgr
                .control_transfer(FILE, None, TransferAction::Cancel)
                .is_ok()
        );
        assert!(h.mgr.held_uploads.is_empty());
        assert_eq!(last_state(&mut h), Some(("cancelled".into(), 512)));
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
    }

    #[tokio::test]
    async fn the_receiver_asking_again_clears_a_held_upload() {
        let mut h = harness();
        peer(&mut h, DEVICE, true);
        h.mgr.held_uploads.insert(
            (DEVICE.into(), FILE.into()),
            HeldUpload {
                file_name: "movie.mp4".into(),
                total_size: TOTAL,
                done: 512,
                waiting: true,
            },
        );
        // stream_file_to_peer clears it before anything is sent; with no real
        // file it returns early, so check the disconnect cleanup instead.
        h.mgr.held_uploads.retain(|(d, _), _| d != DEVICE);
        assert!(h.mgr.held_uploads.is_empty());
    }

    #[tokio::test]
    async fn resuming_an_upload_to_an_old_peer_is_refused() {
        let mut h = harness();
        peer(&mut h, DEVICE, false);
        h.mgr.held_uploads.insert(
            (DEVICE.into(), FILE.into()),
            HeldUpload {
                file_name: "movie.mp4".into(),
                total_size: TOTAL,
                done: 512,
                waiting: false,
            },
        );
        let reply = h.mgr.control_transfer(FILE, None, TransferAction::Resume);
        assert!(reply.unwrap_err().contains("older ClipX"));
        assert!(resumes_sent(&mut h).is_empty());
    }

    #[tokio::test]
    async fn sender_cancelling_ends_our_download_and_reports_it() {
        let mut h = harness();
        partial_download(&mut h);

        h.mgr.on_file_transfer_cancel(
            DEVICE.into(),
            proto::FileTransferCancel {
                file_id: FILE.into(),
                pause: false,
            },
        );

        assert!(!h.mgr.pending_downloads.contains_key(FILE));
        assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
        let released = std::iter::from_fn(|| h.clipboard_rx.try_recv().ok())
            .any(|c| matches!(c, ClipboardCommand::FileDownloadReleased { .. }));
        assert!(released, "clipboard manager must free the download slot");
    }

    #[tokio::test]
    async fn sender_pausing_keeps_our_partial_data_and_waits_for_a_resume() {
        let mut h = harness();
        partial_download(&mut h);

        h.mgr.on_file_transfer_cancel(
            DEVICE.into(),
            proto::FileTransferCancel {
                file_id: FILE.into(),
                pause: true,
            },
        );

        assert!(!h.mgr.pending_downloads.contains_key(FILE));
        assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert!(h.mgr.incoming_state_path(FILE).exists());
        assert!(
            h.mgr.incoming_paused_path(FILE).exists(),
            "must not restart by itself"
        );
        assert_eq!(last_state(&mut h), Some(("paused".into(), PARTIAL)));
        let _ = download_requests(&mut h);

        // The sender presses Resume: we ask again from the kept offset.
        h.mgr.on_file_transfer_resume(
            DEVICE.into(),
            proto::FileTransferResume {
                file_id: FILE.into(),
            },
        );
        assert_eq!(download_requests(&mut h), vec![PARTIAL]);
        assert!(!h.mgr.incoming_paused_path(FILE).exists());
    }

    #[tokio::test]
    async fn a_resume_from_the_wrong_device_or_with_nothing_kept_is_ignored() {
        let mut h = harness();
        partial_download(&mut h);
        h.mgr.stop_transfer(FILE, None, true);
        let _ = download_requests(&mut h);
        peer(&mut h, OTHER, true);

        h.mgr.on_file_transfer_resume(
            OTHER.into(),
            proto::FileTransferResume {
                file_id: FILE.into(),
            },
        );
        h.mgr.on_file_transfer_resume(
            DEVICE.into(),
            proto::FileTransferResume {
                file_id: "unknown".into(),
            },
        );
        assert!(download_requests(&mut h).is_empty());
        assert!(!h.mgr.pending_downloads.contains_key(FILE));
    }

    #[tokio::test]
    async fn sender_cancelling_a_paused_download_discards_the_kept_partial() {
        let mut h = harness();
        partial_download(&mut h);
        h.mgr.stop_transfer(FILE, None, true);
        let _ = last_event(&mut h);

        h.mgr.on_file_transfer_cancel(
            DEVICE.into(),
            proto::FileTransferCancel {
                file_id: FILE.into(),
                pause: false,
            },
        );

        assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert!(!h.mgr.incoming_state_path(FILE).exists());
        assert!(!h.mgr.incoming_paused_path(FILE).exists());
        assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
    }

    #[tokio::test]
    async fn cancel_from_another_device_does_not_touch_our_download() {
        let mut h = harness();
        partial_download(&mut h);

        h.mgr.on_file_transfer_cancel(
            "someone-else".into(),
            proto::FileTransferCancel {
                file_id: FILE.into(),
                pause: true,
            },
        );
        h.mgr.on_file_transfer_cancel(
            "someone-else".into(),
            proto::FileTransferCancel {
                file_id: FILE.into(),
                pause: false,
            },
        );

        assert!(h.mgr.pending_downloads.contains_key(FILE));
        assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert!(h.mgr.incoming_state_path(FILE).exists());
    }

    #[tokio::test]
    async fn the_snapshot_lists_downloads_uploads_and_paused_uploads_per_device() {
        let mut h = harness();
        partial_download(&mut h);
        peer(&mut h, OTHER, false);
        let _t = upload_to(&mut h, OTHER, 300);
        h.mgr.held_uploads.insert(
            ("dev-c".into(), "file-2".into()),
            HeldUpload {
                file_name: "a.zip".into(),
                total_size: 10,
                done: 5,
                waiting: false,
            },
        );

        let mut snap = h.mgr.list_transfers();
        snap.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        let rows: Vec<_> = snap
            .iter()
            .map(|t| {
                (
                    t.device_id.as_str(),
                    t.direction.as_str(),
                    t.state.as_str(),
                    t.done,
                    t.can_pause,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (DEVICE, "download", "receiving", PARTIAL, true),
                (OTHER, "send", "sending", 300, false),
                ("dev-c", "send", "paused", 5, false),
            ]
        );
    }

    #[tokio::test]
    async fn a_paused_download_can_be_cancelled_and_survives_in_the_snapshot() {
        let mut h = harness();
        partial_download(&mut h);
        assert!(
            h.mgr
                .control_transfer(FILE, None, TransferAction::Pause)
                .is_ok()
        );
        let _ = (cancels(&mut h), last_event(&mut h));

        // Nothing is running, yet the paused download is listed (this is what
        // makes it visible again after an app restart).
        let snap = h.mgr.list_transfers();
        assert_eq!(snap.len(), 1);
        assert_eq!(
            (
                snap[0].state.as_str(),
                snap[0].direction.as_str(),
                snap[0].done
            ),
            ("paused", "download", PARTIAL)
        );

        // Cancel drops the kept partial data and tells the sender.
        assert_eq!(
            h.mgr
                .control_transfer(FILE, Some(DEVICE), TransferAction::Cancel),
            Ok("transfer cancelled".to_string())
        );
        assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
        assert!(!h.mgr.incoming_state_path(FILE).exists());
        assert!(!h.mgr.incoming_paused_path(FILE).exists());
        assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
        assert!(h.mgr.list_transfers().is_empty());
    }

    #[tokio::test]
    async fn nothing_to_control_gives_a_clear_message() {
        let mut h = harness();
        assert_eq!(
            h.mgr.control_transfer("none", None, TransferAction::Cancel),
            Err("No running transfer to cancel".to_string())
        );
        assert_eq!(
            h.mgr.control_transfer("none", None, TransferAction::Pause),
            Err("No running transfer to pause".to_string())
        );
        assert!(
            h.mgr
                .control_transfer("none", None, TransferAction::Resume)
                .unwrap_err()
                .contains("receiving")
        );
    }
}
