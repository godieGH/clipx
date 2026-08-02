use super::pairing::{self, PairingDecision};
use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::{DeviceType, SeenDevice};
use crate::message::proto;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

pub struct DeviceManager {
    seen: SeenDeviceRegistry,
    trusted: TrustedDeviceStore,
}

impl DeviceManager {
    pub fn new(trusted_store_path: std::path::PathBuf) -> Self {
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
        }
    }

    pub async fn run(
        mut self,
        mut shutdown_rx: watch::Receiver<bool>,
        mut discovered_rx: mpsc::UnboundedReceiver<(proto::Announce, SocketAddr)>,
    ) {
        let mut prune_interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
                Some((announce, addr)) = discovered_rx.recv() => {
                    self.handle_announce(announce, addr);
                }
                _ = prune_interval.tick() => {
                    self.seen.prune_stale(Duration::from_secs(30));
                }
            }
        }

        tracing::info!("device manager stopped");
    }

    fn handle_announce(&mut self, announce: proto::Announce, addr: SocketAddr) {
        let device_type = match announce.device_type {
            1 => DeviceType::Windows,
            2 => DeviceType::Android,
            _ => {
                tracing::warn!("unknown device_type {} from {addr}", announce.device_type);
                return;
            }
        };

        self.seen.upsert(SeenDevice {
            id: announce.device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            last_seen: Instant::now(),
        });

        if self.trusted.is_trusted(&announce.device_id) {
            tracing::info!("seen trusted device: {} at {addr}", announce.device_name);
        } else {
            match pairing::evaluate(self.seen.get(&announce.device_id).unwrap(), &self.trusted) {
                PairingDecision::AwaitingUserApproval => {
                    tracing::info!("device {} awaiting pairing approval", announce.device_name);
                }
                PairingDecision::AutoApproved => unreachable!(),
            }
        }
    }

    /// For a future CLI/IPC "pair <id>" command.
    pub fn approve_pairing(&mut self, device_id: &str) -> bool {
        let Some(device) = self.seen.get(device_id) else {
            return false; // can't pair with something we haven't seen
        };

        pairing::approve(device, &mut self.trusted);
        true
    }
}
