use super::pairing::{self, PairingDecision};
use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::{DeviceType, SeenDevice};
use crate::device::identity::DeviceIdentity;
use crate::message::proto;
use crate::net::challenge;
use crate::net::pending_requests::PendingRequests;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

pub struct DeviceManager {
    seen: SeenDeviceRegistry,
    trusted: TrustedDeviceStore,
    identity: Arc<DeviceIdentity>,
    socket: Arc<UdpSocket>,
    pending: PendingRequests<proto::ChallengeResponse>,
}

/// Result of a verify_peer call, reported back into the manager's own loop
/// since handle_announce itself can't .await.
struct VerifyOutcome {
    device_id: String,
    device_name: String,
    verified: bool,
}

impl DeviceManager {
    pub fn new(
        trusted_store_path: std::path::PathBuf,
        identity: Arc<DeviceIdentity>,
        socket: Arc<UdpSocket>,
        pending: PendingRequests<proto::ChallengeResponse>,
    ) -> Self {
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
            identity,
            socket,
            pending,
        }
    }

    pub async fn run(
        mut self,
        mut shutdown_rx: watch::Receiver<bool>,
        mut discovered_rx: mpsc::UnboundedReceiver<(proto::Announce, SocketAddr)>,
    ) {
        let mut prune_interval = tokio::time::interval(Duration::from_secs(10));
        let (verify_tx, mut verify_rx) = mpsc::unbounded_channel::<VerifyOutcome>();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
                Some((announce, addr)) = discovered_rx.recv() => {
                    self.handle_announce(announce, addr, &verify_tx);
                }
                _ = prune_interval.tick() => {
                    self.seen.prune_stale(Duration::from_secs(30));
                }
                Some(outcome) = verify_rx.recv() => {
                    if outcome.verified {
                        tracing::info!(
                            "device {} PASSED challenge-response — identity confirmed",
                            outcome.device_name
                        );
                        // next milestone: hand off to transport to open a WS connection
                    } else {
                        tracing::warn!(
                            "device {} FAILED challenge-response — refusing connection",
                            outcome.device_name
                        );
                    }
                }
            }
        }

        tracing::info!("device manager stopped");
    }

    fn handle_announce(
        &mut self,
        announce: proto::Announce,
        addr: SocketAddr,
        verify_tx: &mpsc::UnboundedSender<VerifyOutcome>,
    ) {
        let device_type = match announce.device_type {
            1 => DeviceType::Windows,
            2 => DeviceType::Android,
            _ => {
                tracing::warn!("unknown device_type {} from {addr}", announce.device_type);
                return;
            }
        };

        let Ok(public_key): Result<[u8; 32], _> = announce.public_key.as_slice().try_into() else {
            tracing::warn!("malformed public key from {addr}, ignoring announce");
            return;
        };

        self.seen.upsert(SeenDevice {
            id: announce.device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            last_seen: Instant::now(),
            public_key,
        });

        match self.trusted.get(&announce.device_id) {
            Some(trusted_device) if trusted_device.public_key == public_key => {
                tracing::info!(
                    "seen trusted device: {} at {addr} — sending challenge",
                    announce.device_name
                );

                // verify_peer is async; handle_announce isn't, so we spawn
                // a task and report the outcome back via verify_tx.
                let socket = self.socket.clone();
                let pending = self.pending.clone();
                let identity = self.identity.clone();
                let verify_tx = verify_tx.clone();
                let device_id = announce.device_id.clone();
                let device_name = announce.device_name.clone();

                tokio::spawn(async move {
                    let verified =
                        challenge::verify_peer(&socket, &pending, &identity, addr, public_key)
                            .await;
                    let _ = verify_tx.send(VerifyOutcome {
                        device_id,
                        device_name,
                        verified,
                    });
                });
            }
            Some(_mismatched) => {
                tracing::warn!(
                    "device {} claims a known id but a DIFFERENT public key — possible impersonation, refusing to auto-trust",
                    announce.device_name
                );
            }
            None => {
                match pairing::evaluate(self.seen.get(&announce.device_id).unwrap(), &self.trusted)
                {
                    PairingDecision::AwaitingUserApproval => {
                        tracing::info!("device {} awaiting pairing approval", announce.device_name);
                    }
                    PairingDecision::AutoApproved => unreachable!(),
                }
            }
        }
    }

    /// For a future CLI/IPC "pair <id>" command.
    pub fn approve_pairing(&mut self, device_id: &str) -> bool {
        let Some(device) = self.seen.get(device_id) else {
            return false;
        };

        pairing::approve(device, &mut self.trusted);
        true
    }
}