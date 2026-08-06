use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::SeenDevice;
use crate::device::identity::DeviceIdentity;
use crate::message::proto;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, oneshot};

pub struct DeviceManager {
    seen: SeenDeviceRegistry,
    trusted: TrustedDeviceStore,
    identity: Arc<DeviceIdentity>,
}

pub enum SeenMode {
    All,
    Trusted,
    Untrusted,
}

pub enum DeviceCommands {
    GetSeen {
        mode: SeenMode,
        reply_to: oneshot::Sender<Vec<SeenDevice>>,
    }
}

impl DeviceManager {
    pub fn new(trusted_store_path: std::path::PathBuf, identity: Arc<DeviceIdentity>) -> Self {
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
            identity,
        }
    }

    pub async fn run(
        mut self,
        mut shutdown_rx: watch::Receiver<bool>,
        mut discovered_rx: mpsc::UnboundedReceiver<(proto::Announce, SocketAddr)>,
        mut device_rx: mpsc::UnboundedReceiver<DeviceCommands>
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
                Some(cmd) = device_rx.recv() => {
                    self.handle_device_command(cmd);
                }
            }
        }

        tracing::info!("device manager stopped");
    }

    fn handle_announce(
        &mut self,
        announce: proto::Announce,
        addr: SocketAddr,
    ) {
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

        self.seen.upsert(SeenDevice {
            id: device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            last_seen: Instant::now(),
        });

        match self.trusted.get(&device_id) {
            Some(_trusted_device) => {
                // if the device is in the trusted store then is trusted no need to pair
            }
            None => {}
        }
    }

    /// For a future CLI/IPC "pair <id>" command.
    pub fn approve_pairing(&mut self, device_id: &str) -> bool {
        let Some(device) = self.seen.get(device_id) else {
            return false;
        };

        //pairing::approve(device, &mut self.trusted);
        true
    }
}


// external device handling
impl DeviceManager {
    fn handle_device_command(&self, cmd: DeviceCommands) {
        match cmd {
            DeviceCommands::GetSeen { mode, reply_to } => {
                let devices = self
                    .seen
                    .list()
                    .filter(|device| match mode {
                        SeenMode::All => true,
                        SeenMode::Trusted => self.trusted.is_trusted(&device.id),
                        SeenMode::Untrusted => !self.trusted.is_trusted(&device.id),
                    })
                    .cloned()
                    .collect::<Vec<SeenDevice>>();
                let _ = reply_to.send(devices);
            }
        }
    }
}