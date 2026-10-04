use crate::message::proto::DeviceType;
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

/// Fixed 4xxx codes carried in PreTransportControl — the only vocabulary
/// either side needs to explain why a pair/connect attempt stopped.
pub mod control {
    pub const USER_DENIED: u32 = 4001;
    pub const CODE_MISMATCH: u32 = 4002;
    pub const SIGNATURE_INVALID: u32 = 4003;
    pub const TIMEOUT: u32 = 4004;
    pub const ALREADY_TRUSTED: u32 = 4005;
    pub const UNKNOWN_DEVICE: u32 = 4006;
    pub const PROTOCOL_ERROR: u32 = 4007;
    /// Sent by the responder side of a connect-challenge race to tell the
    /// losing initiator to stand down: a connect request for this device is
    /// already in flight on a different socket, so the caller should drop
    /// its own attempt instead of treating this as a hard failure.
    pub const CONNECT_IN_PROGRESS: u32 = 4008;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Responder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairStage {
    Dialing,               // initiator only: Connect sent, waiting for the socket
    AwaitingPeerResponse,  // initiator: Request sent, waiting Response
    AwaitingLocalApproval, // responder: Request received, waiting user's allow/deny
    AwaitingChallenge,     // responder: Response sent, waiting Challenge
    AwaitingSignature,     // initiator: Challenge sent, waiting ChallengeResponse
    AwaitingCodeConfirm,   // both: waiting local user to confirm the displayed code
    AwaitingAck,           // responder: signed + confirmed, waiting Ack
}

#[derive(Debug)]
pub struct PairSession {
    pub role: Role,
    pub stage: PairStage,
    pub started_at: Instant,
    pub _peer_addr: SocketAddr,
    pub peer_public_key: Option<[u8; 32]>,
    pub peer_name: Option<String>,
    pub peer_device_type: DeviceType,
    pub nonce: Option<[u8; 32]>,
    pub code: Option<u32>,
    /// Held between "we signed" and "user confirmed the code" — released
    /// (sent) only on confirmation, per the design: nothing crosses the
    /// wire until the local user approves what they're looking at.
    pub pending_signature: Option<[u8; 64]>,
    /// initiator only — true once the local user has confirmed the pair code matches
    /// even if the peer signature has not arrived yet. Lets confirmation
    /// and the challange response race — which ever finishes second finalizes
    pub code_confirmed: bool,
}

impl PairSession {
    pub fn new_initiator(addr: SocketAddr, peer_device_type: DeviceType) -> Self {
        Self {
            role: Role::Initiator,
            stage: PairStage::Dialing,
            started_at: Instant::now(),
            _peer_addr: addr,
            peer_public_key: None,
            peer_name: None,
            peer_device_type,
            nonce: None,
            code: None,
            pending_signature: None,
            code_confirmed: false,
        }
    }

    pub fn new_responder(
        addr: SocketAddr,
        peer_public_key: [u8; 32],
        peer_name: String,
        peer_device_type: DeviceType,
    ) -> Self {
        Self {
            role: Role::Responder,
            stage: PairStage::AwaitingLocalApproval,
            started_at: Instant::now(),
            _peer_addr: addr,
            peer_public_key: Some(peer_public_key),
            peer_name: Some(peer_name),
            peer_device_type,
            nonce: None,
            code: None,
            pending_signature: None,
            code_confirmed: false,
        }
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.started_at.elapsed() > ttl
    }

    /// Order-independent so both sides compute the same code regardless of
    /// who's initiator — sorts the two public keys before hashing.
    pub fn compute_code(nonce: &[u8], pk_a: &[u8; 32], pk_b: &[u8; 32]) -> u32 {
        let (first, second) = if pk_a <= pk_b {
            (pk_a, pk_b)
        } else {
            (pk_b, pk_a)
        };
        let mut hasher = Sha256::new();
        hasher.update(nonce);
        hasher.update(first);
        hasher.update(second);
        let digest = hasher.finalize();
        u32::from_be_bytes(digest[0..4].try_into().unwrap()) % 1_000_000
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectStage {
    Dialing,           // initiator only
    AwaitingSignature, // initiator: Challenge sent, waiting ChallengeResponse
    AwaitingAck,       // responder: signed, waiting Ack
}

#[derive(Debug)]
pub struct ConnectSession {
    pub role: Role,
    pub stage: ConnectStage,
    pub started_at: Instant,
    pub peer_addr: SocketAddr,
    pub nonce: Option<[u8; 32]>,
    pub retry_count: u8,
    /// The transport connection this session's handshake is bound to. `None`
    /// while an initiator is still dialing (no socket yet). Once set, every
    /// message for this session must arrive on this exact connection_id —
    /// this is what lets a duplicate simultaneous connection be told apart
    /// from the one actually carrying this handshake.
    pub connection_id: Option<String>,
}

impl ConnectSession {
    pub const MAX_RETRIES: u8 = 2;

    pub fn new_initiator(addr: SocketAddr) -> Self {
        Self {
            role: Role::Initiator,
            stage: ConnectStage::Dialing,
            started_at: Instant::now(),
            peer_addr: addr,
            nonce: None,
            retry_count: 0,
            connection_id: None,
        }
    }

    pub fn new_responder(addr: SocketAddr) -> Self {
        Self {
            role: Role::Responder,
            stage: ConnectStage::AwaitingAck,
            started_at: Instant::now(),
            peer_addr: addr,
            nonce: None,
            retry_count: 0,
            connection_id: None,
        }
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.started_at.elapsed() > ttl
    }
}
