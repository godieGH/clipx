use clipx_core::device::identity::DeviceIdentity;
use clipx_core::message::proto;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Everything captured so far about the peer on one open connection.
/// Nothing here is enforced (unlike the real DeviceManager's PairSession
/// stage machine) — you can send messages out of order on purpose, which is
/// the point of this tool: testing how the real side reacts to that.
#[derive(Default, Debug, Clone)]
pub struct SessionInfo {
    pub peer_public_key: Option<[u8; 32]>,
    pub peer_name: Option<String>,
    pub nonce: Option<[u8; 32]>,
}

pub struct ConnHandle {
    pub addr: SocketAddr,
    pub outbound_tx: mpsc::UnboundedSender<WsMessage>,
    pub session: Mutex<SessionInfo>,
}

/// Broadcast (not mpsc) so both the printer task and any in-flight `pair
/// auto` / `respond auto` waiter can each see every event independently.
#[derive(Debug, Clone)]
pub enum Event {
    Discovered { device_id: String, name: String, addr: SocketAddr },
    Connected { label: String, addr: SocketAddr },
    Disconnected { label: String },
    Message { label: String, message: proto::PeerMessage },
    Info(String),
    Warn(String),
}

pub struct AppState {
    pub identity: Arc<DeviceIdentity>,
    pub device_name: String,
    pub discovered: Mutex<HashMap<String, (proto::Announce, SocketAddr)>>,
    pub conns: Mutex<HashMap<String, Arc<ConnHandle>>>,
    pub events_tx: broadcast::Sender<Event>,
    pub next_inbound: AtomicU32,
    pub auto_respond: AtomicBool,
    pub broadcast_shutdown: Mutex<Option<watch::Sender<bool>>>,
    pub listen_shutdown: Mutex<Option<watch::Sender<bool>>>,
    pub inbound_shutdown: Mutex<Option<watch::Sender<bool>>>,
}

impl AppState {
    pub fn emit(&self, ev: Event) {
        // No active receiver is a normal state right after startup — ignore.
        let _ = self.events_tx.send(ev);
    }
}
