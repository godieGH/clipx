use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::{IdentitySnapshot, SeenDevice, TrustedDevice};
use crate::clipboard::manager::{ClipboardCommand, ClipboardOutbound, ClipboardPayload};
use crate::device::identity::{self, DeviceIdentity};
use crate::device::pairing::{ConnectSession, ConnectStage, PairSession, PairStage, Role};
use crate::device::{config, pairing};
use crate::message::proto::{self, clipboard_message, peer_message::Body};
use crate::netio::transport::{TransportCommand, TransportEvent};
use crate::notification::{platform::NotificationEngine as Engine, PairDecision, Prompt};
use crate::platform::CoreEvent;
use std::{collections::{HashMap, HashSet}, fs::{File, OpenOptions}, io::Write, path::PathBuf};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

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

    // tracks current download file(s) — file_id -> entry_id
    pending_downloads: HashMap<String, String>, 
}

/// Results of a notification popup, fed back into the manager's own select
/// loop — `NotificationEngine::ask()` can take up to its timeout, so it's
/// always spawned as its own task rather than awaited inline; awaiting it
/// directly would stall every other device-manager operation for as long
/// as the popup is on screen.
struct IncomingFileAssembly {
    next_seq: u32,
    total_size: u64,
    received_size: u64,
    temp_path: PathBuf,
    file: File,
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
        let incoming_files_dir = trusted_store_path.parent().unwrap_or_else(|| std::path::Path::new(".")).join("incoming-files");
        let _ = std::fs::create_dir_all(&incoming_files_dir);
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
            identity,
            pair_sessions: HashMap::new(),
            connect_sessions: HashMap::new(),
            connected: HashMap::new(),
            incoming_files: HashMap::new(),
            incoming_files_dir,
            transport_tx,
            notification,
            notify_tx,
            notify_rx,
            clipboard_tx,
            events_tx,
            pending_downloads: HashMap::new(),
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
            Ok(proto::pairing_event::State::Succeeded) => crate::platform::PairingEventState::Succeeded,
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
        let notify_tx = self.notify_tx.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
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
            }
        }
        tracing::info!("device manager stopped");
    }

    /// Dispatches clipboard payloads to every currently connected, trusted peer.
    fn broadcast_clipboard(&self, payload: ClipboardPayload, file_offer: Option<proto::FileOffer>) {
        if self.connected.is_empty() { return; }
        let ids: Vec<String> = self.connected.keys().cloned().collect();
        for device_id in ids {
            let content = match (&payload, file_offer.as_ref()) {
                (ClipboardPayload::Text(text), _) => Some(clipboard_message::Content::Text(text.clone())),
                (ClipboardPayload::RichText { text, html }, _) => Some(clipboard_message::Content::RichText(proto::RichText { text: text.clone(), html: html.clone() })),
                (ClipboardPayload::Image { width, height, rgba }, _) => Some(clipboard_message::Content::Image(proto::ImageContent { width: *width, height: *height, rgba: rgba.clone() })),
                (ClipboardPayload::Files(_), Some(file)) | (ClipboardPayload::FileBytes { .. }, Some(file)) | (ClipboardPayload::FilePath { .. }, Some(file)) => Some(clipboard_message::Content::FileOffer(file.clone())),
                (ClipboardPayload::Files(_), None) | (ClipboardPayload::FileBytes { .. }, None) | (ClipboardPayload::FilePath { .. }, None) => None,
            };
            if let Some(content) = content {
                self.send_peer(&device_id, Body::Clipboard(proto::ClipboardMessage { content: Some(content) }));
            }
        }
    }

    fn handle_clipboard_outbound(&mut self, outbound: ClipboardOutbound, _clipboard_tx: &mpsc::UnboundedSender<ClipboardCommand>) {
        match outbound {
            ClipboardOutbound::Payload(payload) => self.broadcast_clipboard(payload, None),
            ClipboardOutbound::FileOffer { file_id, name, mime_type, size, expires_at_ms } => {
                self.broadcast_clipboard(ClipboardPayload::FileBytes { name: name.clone(), mime_type: mime_type.clone(), data: Vec::new() }, Some(proto::FileOffer { file_id, name, mime_type, size, expires_at_ms }));
            }
            ClipboardOutbound::FileRequest { device_id, file_id, entry_id } => {
                self.pending_downloads.insert(file_id.clone(), entry_id);
                self.send_peer(&device_id, Body::FileDownloadRequest(proto::FileDownloadRequest { file_id }));
            }
            ClipboardOutbound::FileStream { device_id, file_id, path, total_size } => self.stream_file_to_peer(device_id, file_id, path, total_size),
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
        self.notify_pair_events(device_id, proto::pairing_event::State::Started as i32, "pairing started");
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
        self.dial(device_id, addr);
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
            TransportEvent::Connected(dev) => self.on_transport_connected(dev.id),
            TransportEvent::Disconnected(device_id) => self.on_transport_disconnected(device_id),
            TransportEvent::ConnectFailed(device_id) => self.on_connect_failed(device_id),
            TransportEvent::PeerMessage { device_id, message } => {
                self.on_peer_message(device_id, message, notify_tx)
            }
        }
    }

    fn on_transport_connected(&mut self, device_id: String) {
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
            if sess.stage == ConnectStage::Dialing {
                sess.stage = ConnectStage::AwaitingSignature;
                let nonce_vec = self.identity.random_nonce();
                let nonce_arr: [u8; 32] = nonce_vec
                    .as_slice()
                    .try_into()
                    .expect("nonce must be 32 bytes");
                sess.nonce = Some(nonce_arr);
                let own_fp = self.identity.get_this_device_fingerprint();
                self.send_peer(
                    &device_id,
                    Body::ConnectChallenge(proto::PeerConnectChallenge {
                        nonce: nonce_vec,
                        initiator_fingerprint: own_fp.to_vec(),
                    }),
                );
            }
        }
    }

    fn on_transport_disconnected(&mut self, device_id: String) {
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
        message: proto::PeerMessage,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        match message.body {
            Some(Body::PairRequest(req)) => self.on_pair_request(device_id, req, notify_tx),
            Some(Body::PairResponse(res)) => self.on_pair_response(device_id, res, notify_tx),
            Some(Body::PairChallenge(c)) => self.on_pair_challenge(device_id, c, notify_tx),
            Some(Body::PairChallengeResponse(r)) => self.on_pair_challenge_response(device_id, r),
            Some(Body::PairAck(_)) => self.on_pair_ack(device_id),
            Some(Body::ConnectChallenge(c)) => self.on_connect_challenge(device_id, c),
            Some(Body::ConnectChallengeResponse(r)) => {
                self.on_connect_challenge_response(device_id, r)
            }
            Some(Body::ConnectAck(_)) => self.on_connect_ack(device_id),
            Some(Body::Control(ctrl)) => self.on_control(device_id, ctrl),
            Some(Body::Clipboard(msg)) => self.on_clipboard_message(device_id, msg),
            Some(Body::FileDownloadRequest(req)) => { self.on_file_download_request(device_id, req); }
            Some(Body::FileChunk(chunk)) => { self.on_file_chunk(device_id, chunk); }
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
                get_formated_fp(device_id)
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
        self.notify_pair_events(device_id, proto::pairing_event::State::Failed as i32, message);
        self.request_transport_disconnect(device_id);
    }

    // ---------------- Connect: responder side ----------------

    fn on_connect_challenge(&mut self, device_id: String, c: proto::PeerConnectChallenge) {
        if !self.trusted.is_trusted(&device_id) {
            self.send_control(
                &device_id,
                pairing::control::UNKNOWN_DEVICE,
                "not a trusted device",
            );
            self.request_transport_disconnect(&device_id);
            return;
        }
        let Ok(nonce_arr): Result<[u8; 32], _> = c.nonce.as_slice().try_into() else {
            self.abort_connect(
                &device_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed nonce",
            );
            return;
        };
        let sig = self.identity.sign(&nonce_arr);
        let addr = self
            .seen
            .get(&device_id)
            .map(|d| SocketAddr::new(d.addr, d.ws_port as u16))
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

        let mut sess = ConnectSession::new_responder(addr);
        sess.nonce = Some(nonce_arr);
        self.connect_sessions.insert(device_id.clone(), sess);

        self.send_peer(
            &device_id,
            Body::ConnectChallengeResponse(proto::PeerConnectChallengeResponse {
                signature: sig.to_vec(),
            }),
        );
    }

    fn on_connect_ack(&mut self, device_id: String) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            return;
        };
        if sess.role != Role::Responder || sess.stage != ConnectStage::AwaitingAck {
            return;
        }
        self.connected
            .insert(device_id.clone(), "connected".to_string());
        self.connect_sessions.remove(&device_id);
        self.notify_devices_changed();
    }

    // ---------------- Connect: initiator side (continued) ----------------

    fn on_connect_challenge_response(
        &mut self,
        device_id: String,
        r: proto::PeerConnectChallengeResponse,
    ) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            return;
        };
        if sess.role != Role::Initiator || sess.stage != ConnectStage::AwaitingSignature {
            return;
        }
        let Some(trusted_device) = self.trusted.get(&device_id) else {
            self.abort_connect(
                &device_id,
                pairing::control::UNKNOWN_DEVICE,
                "device no longer trusted",
            );
            return;
        };
        let nonce = sess.nonce.unwrap();
        if !identity::verify(&trusted_device.public_key, &nonce, &r.signature) {
            self.abort_connect(
                &device_id,
                pairing::control::SIGNATURE_INVALID,
                "signature verification failed",
            );
            return;
        }
        self.send_peer(&device_id, Body::ConnectAck(proto::ConnectAck {}));
        self.connected
            .insert(device_id.clone(), "connected".to_string());
        self.connect_sessions.remove(&device_id);
        self.notify_devices_changed();
    }

    fn abort_connect(&mut self, device_id: &str, code: u32, message: &str) {
        self.send_control(device_id, code, message);
        self.connect_sessions.remove(device_id);
        self.request_transport_disconnect(device_id);
    }

    // ---------------- Shared control / helpers ----------------

    fn on_control(&mut self, device_id: String, ctrl: proto::PreTransportControl) {
        tracing::warn!(
            "pretransport control from {device_id}: {} ({})",
            ctrl.code,
            ctrl.message
        );
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
        if !self.trusted.is_trusted(&device_id) { tracing::warn!("dropping clipboard content from untrusted device {device_id}"); return; }
        let Some(content) = msg.content else { return; };
        let payload = match content {
            clipboard_message::Content::Text(text) => crate::clipboard::manager::IncomingPayload::Text(text),
            clipboard_message::Content::RichText(value) => crate::clipboard::manager::IncomingPayload::RichText { text: value.text, html: value.html },
            clipboard_message::Content::Image(value) => {
                let expected = (value.width as usize).checked_mul(value.height as usize).and_then(|v| v.checked_mul(4));
                if expected != Some(value.rgba.len()) || value.width == 0 || value.height == 0 || value.rgba.len() > MAX_IMAGE_BYTES {
                    tracing::warn!("rejecting invalid clipboard image from {device_id}");
                    return;
                }
                crate::clipboard::manager::IncomingPayload::Image { width: value.width, height: value.height, rgba: value.rgba }
            },
            clipboard_message::Content::FileOffer(file) => crate::clipboard::manager::IncomingPayload::FileOffer { file },
        };
        let device_name = self.trusted.get(&device_id).map(|d| d.name.clone()).unwrap_or_else(|| device_id.clone());
        let _ = self.clipboard_tx.send(ClipboardCommand::IncomingRemote { payload, source_device_name: device_name, source_device_id: device_id });
    }

    fn on_file_download_request(&mut self, device_id: String, req: proto::FileDownloadRequest) {
        let _ = self.clipboard_tx.send(ClipboardCommand::PeerFileDownloadRequest { device_id, file_id: req.file_id });
    }

    fn on_file_chunk(&mut self, device_id: String, chunk: proto::FileChunk) {
        if uuid::Uuid::parse_str(&chunk.file_id).is_err() { return; }
        let entry_id = self.pending_downloads.get(&chunk.file_id).cloned().unwrap_or_else(|| chunk.file_id.clone());
        const MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024;
        if chunk.total_size > MAX_FILE_SIZE || chunk.data.len() as u64 > MAX_FILE_SIZE {
            self.pending_downloads.remove(&chunk.file_id);
            let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: 0, total: chunk.total_size,
                state: "failed".into(), message: "File exceeds the 512 MiB transfer limit".into(),
            });
            return;
        }
        let key = (device_id.clone(), chunk.file_id.clone());

        if chunk.seq == 0 {
            if let Some(old) = self.incoming_files.remove(&key) {
                let _ = std::fs::remove_file(old.temp_path);
            }
            let incoming_dir = self.incoming_files_dir.clone();
            if let Err(e) = std::fs::create_dir_all(&incoming_dir) {
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: 0, total: chunk.total_size,
                    state: "failed".into(), message: format!("Could not prepare incoming storage: {e}"),
                });
                return;
            }
            if chunk.data.len() as u64 > chunk.total_size {
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: 0, total: chunk.total_size,
                    state: "failed".into(), message: "Received more data than advertised".into(),
                });
                return;
            }
            let temp_path = incoming_dir.join(format!("clipx-{}-{}", device_id, chunk.file_id));
            let Ok(mut file) = OpenOptions::new().create_new(true).write(true).open(&temp_path) else {
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: 0, total: chunk.total_size,
                    state: "failed".into(), message: "Could not create incoming file".into(),
                });
                return;
            };
            if let Err(e) = file.write_all(&chunk.data) {
                let _ = std::fs::remove_file(&temp_path);
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: 0, total: chunk.total_size,
                    state: "failed".into(), message: format!("Could not write incoming file: {e}"),
                });
                return;
            }
            let received = chunk.data.len() as u64;
            if chunk.eof {
                if received != chunk.total_size {
                    let _ = std::fs::remove_file(&temp_path);
                    self.pending_downloads.remove(&chunk.file_id);
                    let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                        entry_id: entry_id.clone(), file_id: chunk.file_id.clone(), done: received, total: chunk.total_size,
                        state: "failed".into(), message: "File ended before all data arrived".into(),
                    });
                    return;
                }
                let _ = file.sync_all();
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.clipboard_tx.send(ClipboardCommand::IncomingFilePath {
                    source_device_id: device_id, file_id: chunk.file_id, path: temp_path,
                });
                return;
            }
            self.incoming_files.insert(key, IncomingFileAssembly {
                next_seq: 1,
                total_size: chunk.total_size,
                received_size: received,
                temp_path,
                file,
            });
            let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                entry_id: entry_id.clone(), file_id: chunk.file_id, done: received, total: chunk.total_size,
                state: "receiving".into(), message: "Receiving file".into(),
            });
            return;
        }

        let (expected_seq, total_size, received_size) = match self.incoming_files.get(&key) {
            Some(state) => (state.next_seq, state.total_size, state.received_size),
            None => {
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id, done: 0, total: chunk.total_size,
                    state: "failed".into(), message: "Incoming file session is missing".into(),
                });
                return;
            }
        };
        let chunk_len = chunk.data.len() as u64;
        if chunk.seq != expected_seq || chunk.total_size != total_size || received_size.saturating_add(chunk_len) > total_size {
            if let Some(old) = self.incoming_files.remove(&key) { let _ = std::fs::remove_file(old.temp_path); }
            self.pending_downloads.remove(&chunk.file_id);
            let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                entry_id: entry_id.clone(), file_id: chunk.file_id, done: received_size, total: total_size,
                state: "failed".into(), message: "Invalid file chunk sequence or size".into(),
            });
            return;
        }
        let write_ok = self.incoming_files.get_mut(&key).map(|state| state.file.write_all(&chunk.data).is_ok()).unwrap_or(false);
        if !write_ok {
            if let Some(old) = self.incoming_files.remove(&key) { let _ = std::fs::remove_file(old.temp_path); }
            self.pending_downloads.remove(&chunk.file_id);
            let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                entry_id: entry_id.clone(), file_id: chunk.file_id, done: received_size, total: total_size,
                state: "failed".into(), message: "Could not write incoming file".into(),
            });
            return;
        }
        let done = received_size + chunk_len;
        if chunk.eof {
            if done != total_size {
                if let Some(old) = self.incoming_files.remove(&key) { let _ = std::fs::remove_file(old.temp_path); }
                self.pending_downloads.remove(&chunk.file_id);
                let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: entry_id.clone(), file_id: chunk.file_id, done, total: total_size,
                    state: "failed".into(), message: "File ended before all data arrived".into(),
                });
                return;
            }
            let Some(done_state) = self.incoming_files.remove(&key) else { return; };
            let _ = done_state.file.sync_all();
            self.pending_downloads.remove(&chunk.file_id);
            let _ = self.clipboard_tx.send(ClipboardCommand::IncomingFilePath { source_device_id: device_id, file_id: chunk.file_id, path: done_state.temp_path });
            return;
        }
        if let Some(state) = self.incoming_files.get_mut(&key) {
            state.next_seq = state.next_seq.saturating_add(1);
            state.received_size = done;
        }

        let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: entry_id.clone(), file_id: chunk.file_id, done, total: total_size,
            state: "receiving".into(), message: "Receiving file".into(),
        });
    }

    fn stream_file_to_peer(&self, device_id: String, file_id: String, path: PathBuf, total_size: u64) {
        let Some(transport_tx) = self.transport_tx.as_ref().cloned() else {
            return;
        };
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            const CHUNK_SIZE: usize = 1024 * 1024;
            const QUEUE_TIMEOUT: Duration = Duration::from_secs(15);
            let mut file = match tokio::fs::File::open(&path).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = events_tx.send(CoreEvent::FileTransferChanged {
                        entry_id: file_id.clone(), file_id: file_id.clone(), done: 0, total: total_size,
                        state: "failed".into(), message: format!("Could not open file: {e}"),
                    });
                    return;
                }
            };
            let mut buf = vec![0u8; CHUNK_SIZE];
            let mut seq = 0u32;
            let mut sent = 0u64;
            loop {
                let n = match file.read(&mut buf).await {
                    Ok(n) => n,
                    Err(e) => {
                        let _ = events_tx.send(CoreEvent::FileTransferChanged {
                            entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                            state: "failed".into(), message: format!("Could not read file: {e}"),
                        });
                        return;
                    }
                };
                if n == 0 {
                    if sent == 0 && total_size == 0 {
                        let body = Body::FileChunk(proto::FileChunk {
                            file_id: file_id.clone(), seq: 0, data: Vec::new(), eof: true, total_size: 0,
                        });
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if transport_tx.send(TransportCommand::SendPeerMessage {
                            device_id: device_id.clone(),
                            message: proto::PeerMessage { body: Some(body) },
                            reply_to: reply_tx,
                        }).is_err() {
                            let _ = events_tx.send(CoreEvent::FileTransferChanged {
                                entry_id: file_id.clone(), file_id: file_id.clone(), done: 0, total: 0,
                                state: "failed".into(), message: "Transport is unavailable".into(),
                            });
                            return;
                        }
                        if !matches!(tokio::time::timeout(QUEUE_TIMEOUT, reply_rx).await, Ok(Ok(true))) {
                            let _ = events_tx.send(CoreEvent::FileTransferChanged {
                                entry_id: file_id.clone(), file_id: file_id.clone(), done: 0, total: 0,
                                state: "failed".into(), message: "Timed out while queuing empty file".into(),
                            });
                            return;
                        }
                        let _ = events_tx.send(CoreEvent::FileTransferChanged {
                            entry_id: file_id.clone(), file_id: file_id.clone(), done: 0, total: 0,
                            state: "complete".into(), message: "File sent".into(),
                        });
                        break;
                    }
                    let _ = events_tx.send(CoreEvent::FileTransferChanged {
                        entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                        state: "failed".into(), message: "Source file ended before the advertised size".into(),
                    });
                    return;
                }
                let end_pos = sent.saturating_add(n as u64);
                if end_pos > total_size {
                    let _ = events_tx.send(CoreEvent::FileTransferChanged {
                        entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                        state: "failed".into(), message: "Source file grew beyond the offered size".into(),
                    });
                    return;
                }
                let eof = end_pos == total_size;
                let body = Body::FileChunk(proto::FileChunk {
                    file_id: file_id.clone(), seq, data: buf[..n].to_vec(), eof, total_size,
                });
                let (reply_tx, reply_rx) = oneshot::channel();
                if transport_tx.send(TransportCommand::SendPeerMessage {
                    device_id: device_id.clone(),
                    message: proto::PeerMessage { body: Some(body) },
                    reply_to: reply_tx,
                }).is_err() {
                    let _ = events_tx.send(CoreEvent::FileTransferChanged {
                        entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                        state: "failed".into(), message: "Transport is unavailable".into(),
                    });
                    return;
                }
                match tokio::time::timeout(QUEUE_TIMEOUT, reply_rx).await {
                    Ok(Ok(true)) => {}
                    Ok(Ok(false)) => {
                        let _ = events_tx.send(CoreEvent::FileTransferChanged {
                            entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                            state: "failed".into(), message: "Transport rejected the file chunk".into(),
                        });
                        return;
                    }
                    _ => {
                        let _ = events_tx.send(CoreEvent::FileTransferChanged {
                            entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                            state: "failed".into(), message: "Timed out while queuing file data".into(),
                        });
                        return;
                    }
                }
                sent = end_pos;
                let _ = events_tx.send(CoreEvent::FileTransferChanged {
                    entry_id: file_id.clone(), file_id: file_id.clone(), done: sent, total: total_size,
                    state: if eof { "complete" } else { "sending" }.into(),
                    message: if eof { "File sent" } else { "Sending file" }.into(),
                });
                if eof { break; }
                seq = seq.saturating_add(1);
            }
        });
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

    fn send_control(&self, device_id: &str, code: u32, message: impl Into<String>) {
        self.send_peer(
            device_id,
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

fn get_formated_fp(device_id: &str) -> String {
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
