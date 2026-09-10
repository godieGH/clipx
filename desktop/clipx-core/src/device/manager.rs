use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::{IdentitySnapshot, SeenDevice, TrustedDevice};
use crate::clipboard::manager::{ClipboardCommand, ClipboardOutbound, ClipboardPayload};
use crate::device::identity::{self, DeviceIdentity};
use crate::device::pairing::{ConnectSession, ConnectStage, PairSession, PairStage, Role};
use crate::device::{config, pairing};
use crate::message::proto::{self, clipboard_message, peer_message::Body};
use crate::netio::transport::{TransportCommand, TransportEvent};
use crate::notification::{PairDecision, Prompt, platform::NotificationEngine as Engine};
use crate::platform::CoreEvent;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_RICH_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_CHUNK_BYTES: usize = 1024 * 1024;
const RESUME_STATE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const STALE_INCOMING_CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

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
    upload_done_tx: mpsc::UnboundedSender<(String, String)>,
    upload_done_rx: mpsc::UnboundedReceiver<(String, String)>,
    resume_candidates: HashMap<String, ResumeState>,
    transport_connections: HashMap<String, String>,
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
}

#[allow(unused)]
struct IncomingFileAssembly {
    next_seq: u64,
    total_size: u64,
    received_size: u64,
    temp_path: PathBuf,
    state_path: PathBuf,
    file: File,
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

struct UploadSession {
    cancel: CancellationToken,
    interrupt: CancellationToken,
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

pub enum SeenMode {
    All,
    Trusted,
    Untrusted,
}

pub struct PendingPairEntry {
    pub device_id: String,
    pub device_name: String,
    pub stage: &'static str,
    pub code: Option<String>,
}

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
        let incoming_files_dir = trusted_store_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("incoming-files");
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
            upload_done_tx,
            upload_done_rx,
            resume_candidates: HashMap::new(),
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
        let mut prune_interval = tokio::time::interval(Duration::from_secs(10));
        let mut incoming_cleanup_interval = tokio::time::interval(STALE_INCOMING_CLEANUP_INTERVAL);
        incoming_cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        self.load_resume_candidates();
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
                    // Availability (Unavailable <-> Disconnected) is derived
                    // from `seen` at read time in build_paired_devices, so a
                    // trusted device quietly timing out here needs its own
                    // ping — nothing else would ever announce it.
                    let trusted_seen_before: HashSet<String> = self
                        .trusted
                        .list()
                        .filter(|d| self.seen.get(&d.id).is_some())
                        .map(|d| d.id.clone())
                        .collect();
                    self.seen.prune_stale(Duration::from_secs(10));
                    let trusted_seen_after: HashSet<String> = self
                        .trusted
                        .list()
                        .filter(|d| self.seen.get(&d.id).is_some())
                        .map(|d| d.id.clone())
                        .collect();
                    if trusted_seen_before != trusted_seen_after {
                        self.notify_devices_changed();
                    }
                    self.prune_expired_sessions();
                }
                Some(cmd) = device_rx.recv() => { self.handle_device_command(cmd).await; }
                Some(event) = peer_event_rx.recv() => { self.handle_transport_event(event, &notify_tx); }
                Some(result) = self.notify_rx.recv() => { self.handle_notify_result(result); }
                // The clipboard component decided its content changed and
                // handed us plain text — dispatch is entirely our call, it
                // never knows this happened.
                Some(outbound) = clipboard_out_rx.recv() => { self.handle_clipboard_outbound(outbound, &clipboard_tx); }
                Some((device_id, file_id)) = self.upload_done_rx.recv() => { self.upload_sessions.remove(&(device_id, file_id)); }
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
                self.pending_downloads.insert(
                    file_id.clone(),
                    PendingDownload {
                        device_id: device_id.clone(),
                        entry_id: entry_id.clone(),
                        file_name: file_name.clone(),
                        total_size,
                        offer_expires_at_ms,
                        received_size: 0,
                        received_complete: false,
                    },
                );
                self.notify_transfer(
                    &entry_id,
                    &file_id,
                    &file_name,
                    "download",
                    0,
                    total_size,
                    "requesting",
                    format!("Downloading file"),
                );
                self.send_peer(
                    &device_id,
                    Body::FileDownloadRequest(proto::FileDownloadRequest { file_id, offset: 0 }),
                );
            }
            ClipboardOutbound::FileDownloadFinished { file_id } => {
                self.pending_downloads.remove(&file_id);
            }
            ClipboardOutbound::FileTransferCancel { device_id, file_id } => {
                self.cancel_local_transfer(&file_id);
                self.send_peer(
                    &device_id,
                    Body::FileTransferCancel(proto::FileTransferCancel { file_id }),
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
        }

        self.seen.upsert(SeenDevice {
            id: device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            ws_port: announce.ws_port,
            last_seen: Instant::now(),
        });

        // A trusted device just came back into view — its connection state
        // flips Unavailable -> Disconnected even though nothing else here
        // changed, so that needs its own ping too.
        if just_appeared && self.trusted.is_trusted(&device_id) {
            self.notify_devices_changed();
        }

        // Auto-connect is deliberately a *fresh discovery* trigger. Do not
        // reconnect merely because an already-visible peer disconnected, and
        // do not start a connection just because the user toggled this on.
        // Both sides are allowed to initiate here; the transport/handshake
        // resolves an overlap instead of using a fingerprint tie-breaker.
        if just_appeared {
            if let Some(td) = self.trusted.get(&device_id) {
                if td.auto_connect
                    && !self.connected.contains_key(&device_id)
                    && !self.connect_sessions.contains_key(&device_id)
                {
                    let _ = self.handle_connect(&device_id);
                }
            }
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

        self.connect_sessions
            .insert(device_id.to_string(), ConnectSession::new_initiator(addr));

        // The peer may already have opened the transport while this logical
        // connect request was being created. Reuse that live connection rather
        // than opening another socket. This is still gated by the caller's
        // normal fresh-discovery/manual-connect rules.
        if let Some(connection_id) = self.transport_connections.get(device_id).cloned() {
            self.on_transport_connected(device_id.to_string(), connection_id);
        } else {
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
            } => {
                if self
                    .connected
                    .get(&device_id)
                    .is_some_and(|current| current != &connection_id)
                {
                    return;
                }
                self.on_peer_message(device_id, connection_id, message, notify_tx)
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
                    }),
                );
            }
            return;
        }
        if let Some(sess) = self.connect_sessions.get_mut(&device_id) {
            if sess.role == Role::Initiator && sess.stage == ConnectStage::Dialing {
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
                    }),
                );
            }
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
            self.incoming_files
                .remove(&(device_id.clone(), file_id.clone()));
            self.notify_transfer(
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
            self.notify_devices_changed();
        }
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
        if self
            .connected
            .get(&device_id)
            .is_some_and(|current| current != &connection_id)
        {
            return;
        }
        match message.body {
            Some(Body::PairRequest(req)) => self.on_pair_request(device_id, req, notify_tx),
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
            None => tracing::warn!("empty PeerMessage from {device_id}"),
        }
    }

    // ---------------- Pair: responder side ----------------

    fn on_pair_request(
        &mut self,
        device_id: String,
        req: proto::PeerPairRequest,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        if self.trusted.is_trusted(&device_id) {
            self.send_control(
                &device_id,
                pairing::control::ALREADY_TRUSTED,
                "already trusted",
            );
            return;
        }
        let Ok(pubkey): Result<[u8; 32], _> = req.public_key.as_slice().try_into() else {
            self.send_control(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed public key",
            );
            return;
        };
        if hex::encode(DeviceIdentity::get_fingerprint_for(pubkey)) != device_id {
            self.send_control(
                &device_id,
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
                pairing::control::SIGNATURE_INVALID,
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
            }),
        );
    }

    fn on_connect_ack(&mut self, device_id: String, connection_id: String) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            return;
        };
        if sess.role != Role::Responder
            || sess.stage != ConnectStage::AwaitingAck
            || sess.connection_id.as_deref() != Some(connection_id.as_str())
        {
            return;
        }
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
                pairing::control::SIGNATURE_INVALID,
                "signature verification failed",
            );
            return;
        }
        self.send_peer_on_connection(
            &device_id,
            &connection_id,
            Body::ConnectAck(proto::ConnectAck {}),
        );
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
            "pretransport control from {device_id}: {} ({})",
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
        let Some(pending) = self.pending_downloads.get(&chunk.file_id).cloned() else {
            return;
        };
        if pending.device_id != device_id || pending.received_complete {
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
        let digest = Sha256::digest(&chunk.data);
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
            if state
                .file
                .seek(SeekFrom::Start(state.received_size))
                .is_err()
                || state.file.write_all(&chunk.data).is_err()
                || state.file.sync_all().is_err()
            {
                self.abort_download(&chunk.file_id, "Could not persist incoming file data");
                return;
            }
            state.received_size.saturating_add(chunk.data.len() as u64)
        };
        if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
            active.received_size = new_offset;
        }
        if let Some(active) = self.pending_downloads.get(&chunk.file_id) {
            self.persist_resume_state(&device_id, &chunk.file_id, active, new_offset);
        }
        self.send_peer(
            &device_id,
            Body::FileChunkAck(proto::FileChunkAck {
                file_id: chunk.file_id.clone(),
                confirmed_offset: new_offset,
            }),
        );
        if chunk.eof {
            if new_offset != pending.total_size {
                self.abort_download(&chunk.file_id, "File ended before all data arrived");
                return;
            }
            let Some(done_state) = self.incoming_files.remove(&key) else {
                return;
            };
            let _ = fs::remove_file(&done_state.state_path);
            if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
                active.received_complete = true;
            }
            self.notify_transfer(
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
        if let Some(session) = self.upload_sessions.get(&(device_id, ack.file_id)) {
            let _ = session.ack_tx.send(ack.confirmed_offset);
        }
    }

    fn on_file_transfer_cancel(&mut self, device_id: String, cancel: proto::FileTransferCancel) {
        if let Some(session) = self
            .upload_sessions
            .get(&(device_id, cancel.file_id.clone()))
        {
            session.cancel.cancel();
        }
        self.cancel_local_download(&cancel.file_id);
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
            },
        );
        self.notify_transfer(
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
        if self.upload_sessions.contains_key(&key) {
            return;
        }
        let Some(connection_id) = self.connected.get(&device_id).cloned() else {
            return;
        };
        let cancel = CancellationToken::new();
        let interrupt = CancellationToken::new();
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
        self.upload_sessions.insert(
            key.clone(),
            UploadSession {
                cancel: cancel.clone(),
                interrupt: interrupt.clone(),
                ack_tx,
            },
        );
        let done_tx = self.upload_done_tx.clone();
        let events_tx = self.events_tx.clone();
        let notification = self.notification.clone();
        let connection_id_for_task = connection_id.clone();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let confirmed_offset = Arc::new(AtomicU64::new(start_offset));
        let confirmed_offset_task = confirmed_offset.clone();
        tokio::spawn(async move {
            let result: Result<(),String> = async {
                let mut file=tokio::fs::File::open(&path).await.map_err(|e|format!("Could not open file: {e}"))?;
                if file.metadata().await.map_err(|e|e.to_string())?.len()!=total_size || start_offset>total_size { return Err("Source file changed or resume offset is invalid".into()); }
                file.seek(SeekFrom::Start(start_offset)).await.map_err(|e|e.to_string())?;
                let mut offset=start_offset; let mut seq=offset/MAX_FILE_CHUNK_BYTES as u64;
                let _ = Self::emit_transfer(&events_tx, &notification, &file_id, &file_name, "send", offset, total_size, "sending", "Sending file".into()).await;
                loop {
                    if interrupt.is_cancelled(){return Err("INTERRUPTED".into());} if cancel.is_cancelled(){return Err("CANCELLED".into());}
                    let mut data=vec![0u8;MAX_FILE_CHUNK_BYTES]; let n=file.read(&mut data).await.map_err(|e|e.to_string())?; if n==0 && offset!=total_size {return Err("Source file ended before the advertised size".into());}
                    data.truncate(n); let end=offset.saturating_add(n as u64); let eof=end==total_size; let digest=Sha256::digest(&data).to_vec();
                    let (reply_tx,reply_rx)=oneshot::channel(); transport_tx.send(TransportCommand::SendPeerMessageOnConnection{device_id:device_id.clone(),connection_id:connection_id_for_task.clone(),message:proto::PeerMessage{body:Some(Body::FileChunk(proto::FileChunk{file_id:file_id.clone(),seq,data,eof,total_size,offset,sha256:digest}))},reply_to:reply_tx}).map_err(|_|"Transport is unavailable".to_string())?;
                    tokio::select!{ _=cancel.cancelled()=>return Err("CANCELLED".into()), _=interrupt.cancelled()=>return Err("INTERRUPTED".into()), r=tokio::time::timeout(Duration::from_secs(15),reply_rx)=>{if !matches!(r,Ok(Ok(true))){return Err("Transport rejected or timed out queuing file data".into());}} }
                    tokio::select!{ _=cancel.cancelled()=>return Err("CANCELLED".into()), _=interrupt.cancelled()=>return Err("INTERRUPTED".into()), ack=tokio::time::timeout(Duration::from_secs(30),ack_rx.recv())=>{match ack{Ok(Some(v)) if v==end=>{},Ok(Some(v))=>return Err(format!("Unexpected transfer acknowledgement offset {v}")),_=>return Err("Timed out waiting for transfer acknowledgement".into())}}}
                    offset=end; confirmed_offset_task.store(offset, Ordering::Release); let state=if eof{"complete"}else{"sending"}; let msg=if eof{format!("Sending finished: {file_name}")}else{format!("Sending {file_name} · {}%",Self::percentage(offset,total_size))}; Self::emit_transfer(&events_tx,&notification,&file_id,&file_name,"send",offset,total_size,state,msg).await; if eof{break;} seq=seq.saturating_add(1);
                } Ok(())
            }.await;
            if let Err(error) = result {
                if error == "CANCELLED" {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed_offset.load(Ordering::Acquire),
                        total_size,
                        "cancelled",
                        format!("Sending {file_name} was cancelled"),
                    )
                    .await;
                } else if error == "INTERRUPTED"
                    || error.starts_with("Transport")
                    || error.starts_with("Timed out")
                {
                    let confirmed = confirmed_offset.load(Ordering::Acquire);
                    Self::emit_transfer(&events_tx, &notification, &file_id, &file_name, "send", confirmed, total_size, "interrupted", format!("Sending {file_name} was interrupted; it can resume from {confirmed} bytes")).await;
                } else {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
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
            let _ = done_tx.send(key);
        });
    }

    fn abort_download(&mut self, file_id: &str, message: impl Into<String>) {
        let pending = self.pending_downloads.remove(file_id);
        if let Some(p) = pending {
            self.incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
                .map(|s| {
                    let _ = fs::remove_file(s.temp_path);
                });
            self.resume_candidates.remove(file_id);
            let _ = fs::remove_file(self.incoming_state_path(file_id));
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
            self.notify_transfer(
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
        if let Some(p) = self.pending_downloads.remove(file_id) {
            self.incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
                .map(|s| {
                    let _ = fs::remove_file(s.temp_path);
                });
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
        if let Ok(bytes) = serde_json::to_vec(&state) {
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(tmp, path);
            }
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
                if let Ok(bytes) = fs::read(&path) {
                    if let Ok(state) = serde_json::from_slice::<ResumeState>(&bytes) {
                        if state.updated_at_ms >= cutoff
                            && state.offer_expires_at_ms >= now
                            && !self
                                .incoming_files_dir
                                .join(format!("clipx-{}.cancelled", state.file_id))
                                .exists()
                        {
                            continue;
                        }
                        let _ = fs::remove_file(
                            self.incoming_temp_path(&state.device_id, &state.file_id),
                        );
                    }
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
            "complete" | "failed" | "expired" | "cancelled" | "interrupted"
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
        });
        let notification = self.notification.clone();
        let file_id = file_id.to_string();
        let file_name = file_name.to_string();
        let direction = direction.to_string();
        let state = state.to_string();
        tokio::spawn(async move {
            notification
                .notify_file_transfer(file_id, file_name, direction, done, total, state, message)
                .await;
        });
    }

    async fn emit_transfer(
        events_tx: &broadcast::Sender<CoreEvent>,
        notification: &E,
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
            )
            .await;
    }

    fn percentage(done: u64, total: u64) -> u8 {
        if total == 0 {
            0
        } else {
            ((done.saturating_mul(100)) / total).min(100) as u8
        }
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
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::SendPeerMessage {
            device_id: device_id.to_string(),
            message: proto::PeerMessage { body: Some(body) },
            reply_to: reply_tx,
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
                devices.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
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
                    .filter_map(|id| self.seen.get(id).cloned())
                    .collect::<Vec<SeenDevice>>();
                let _ = reply_to.send(devices);
            }
            DeviceCommands::GetIdentity { reply_to } => {
                let addr = config::preferred_local_ip().unwrap_or(Ipv4Addr::UNSPECIFIED);
                let _ = reply_to.send(IdentitySnapshot {
                    device_id: hex::encode(self.identity.get_this_device_fingerprint()),
                    device_name: config::get_hostname(),
                    public_key_hex: hex::encode(self.identity.public_key_bytes()),
                    ws_port: config::get_ws_port(),
                    device_type: config::get_current_device_type(),
                    ip_addr: addr.to_string(),
                });
            }
            DeviceCommands::GetPaired { reply_to } => {
                let _ = reply_to.send(self.build_paired_devices().await);
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
