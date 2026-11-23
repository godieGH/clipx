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

// Must stay above the `mod` lines: macro_rules! is textually scoped.
macro_rules! shorter {
    ($s:expr) => {{
        let mut k = String::from($s);
        k.truncate(16);
        k.push_str("...");
        k
    }};
}

mod clipboard;
mod commands;
mod connection;
mod download;
mod pair;
mod resume;
#[cfg(test)]
mod tests;
mod transfer_control;
mod transport;
mod upload;

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

/// Results of a notification popup, fed back into the manager's own select
/// loop — `NotificationEngine::ask()` can take up to its timeout, so it's
/// always spawned as its own task rather than awaited inline; awaiting it
/// directly would stall every other device-manager operation for as long
/// as the popup is on screen.
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
}
