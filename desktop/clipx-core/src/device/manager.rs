use super::seen::SeenDeviceRegistry;
use super::trusted::TrustedDeviceStore;
use super::types::{IdentitySnapshot, SeenDevice};
use crate::device::identity::DeviceIdentity;
use crate::device::pairing;
use crate::message::proto;
use crate::net::transport::TransportCommand;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, oneshot};

pub struct DeviceManager {
    seen: SeenDeviceRegistry,
    trusted: TrustedDeviceStore,
    identity: Arc<DeviceIdentity>,
    pending_pairings: HashMap<String, String>,
    connected: HashMap<String, String>,
    transport_tx: Option<mpsc::UnboundedSender<TransportCommand>>,
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
    },
    Pair {
        device_id: String,
        reply_to: oneshot::Sender<String>,
    },
    PendingPairings {
        reply_to: oneshot::Sender<Vec<(String, String)>>,
    },
    ApprovePairing {
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
}

impl DeviceManager {
    pub fn new(
        trusted_store_path: std::path::PathBuf,
        identity: Arc<DeviceIdentity>,
        transport_tx: Option<mpsc::UnboundedSender<TransportCommand>>,
    ) -> Self {
        Self {
            seen: SeenDeviceRegistry::new(),
            trusted: TrustedDeviceStore::load(trusted_store_path),
            identity,
            pending_pairings: HashMap::new(),
            connected: HashMap::new(),
            transport_tx,
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

    fn request_transport_connect(&self, device_id: &str) {
        if let Some(tx) = self.transport_tx.as_ref() {
            let (reply_tx, _reply_rx) = oneshot::channel();
            let _ = tx.send(TransportCommand::Connect {
                device_id: device_id.to_string(),
                reply_to: reply_tx,
            });
        }
    }

    fn handle_pair_request(&mut self, device_id: &str) -> String {
        if self.trusted.is_trusted(device_id) {
            return "already trusted".to_string();
        }

        if let Some(_device) = self.seen.get(device_id) {
            let challenge = pairing::create_challenge(&self.identity, &self.identity.public_key_bytes());
            let response = pairing::respond_to_challenge(&self.identity, &challenge);
            let code = pairing::pairing_code(&challenge, &response);
            self.pending_pairings.insert(device_id.to_string(), code.clone());
            return format!("pairing requested for {device_id}; code {code}");
        }

        "device not found".to_string()
    }

    fn handle_approve_pairing(&mut self, device_id: &str) -> String {
        match self.pending_pairings.remove(device_id) {
            Some(code) => {
                self.trusted.trust(super::types::TrustedDevice {
                    id: device_id.to_string(),
                    name: device_id.to_string(),
                    device_type: crate::message::proto::DeviceType::Unspecified,
                    paired_at: std::time::SystemTime::now(),
                    public_key: [0u8; 32],
                });
                self.connected.insert(device_id.to_string(), code.clone());
                self.request_transport_connect(device_id);
                format!("pairing approved; code {code}")
            }
            None => "no pending pairing".to_string(),
        }
    }

    fn handle_connect(&mut self, device_id: &str) -> String {
        if !self.trusted.is_trusted(device_id) {
            return "device is not trusted".to_string();
        }

        self.connected.insert(device_id.to_string(), "connected".to_string());
        self.request_transport_connect(device_id);
        "connected".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_requests_transport_for_trusted_device() {
        let temp_dir = std::env::temp_dir().join("clipx-test-device-manager");
        let identity = Arc::new(DeviceIdentity::load_or_create(temp_dir.join("identity")));
        let (transport_tx, mut transport_rx) = mpsc::unbounded_channel();
        let manager = DeviceManager::new(temp_dir.join("trusted.json"), identity, Some(transport_tx));

        manager.request_transport_connect("device-1");

        let command = transport_rx.blocking_recv().expect("transport command should be sent");
        match command {
            TransportCommand::Connect { device_id, .. } => assert_eq!(device_id, "device-1"),
            other => panic!("expected connect command, got {other:?}"),
        }
    }
}


// external device handling
impl DeviceManager {
    fn handle_device_command(&mut self, cmd: DeviceCommands) {
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
            DeviceCommands::Pair { device_id, reply_to } => {
                let _ = reply_to.send(self.handle_pair_request(&device_id));
            }
            DeviceCommands::PendingPairings { reply_to } => {
                let pending = self.pending_pairings.iter().map(|(id, code)| (id.clone(), code.clone())).collect();
                let _ = reply_to.send(pending);
            }
            DeviceCommands::ApprovePairing { device_id, reply_to } => {
                let _ = reply_to.send(self.handle_approve_pairing(&device_id));
            }
            DeviceCommands::Connect { device_id, reply_to } => {
                let _ = reply_to.send(self.handle_connect(&device_id));
            }
            DeviceCommands::Connected { reply_to } => {
                let devices = self
                    .connected
                    .keys()
                    .filter_map(|device_id| self.seen.get(device_id).cloned())
                    .collect::<Vec<SeenDevice>>();
                let _ = reply_to.send(devices);
            }
            DeviceCommands::GetIdentity { reply_to } => {
                let _ = reply_to.send(IdentitySnapshot {
                    device_id: hex::encode(self.identity.get_this_device_fingerprint()),
                    device_name: crate::device::config::get_hostname(),
                    public_key_hex: hex::encode(self.identity.public_key_bytes()),
                    ws_port: crate::device::config::get_ws_port(),
                    device_type: crate::device::config::get_current_device_type(),
                    // the device manager should ask the network component for what addr this device is bound
                    // for now since this is prototype the device is bound to 0.0.0.0 all interfaces as long as they send to the bound ws_port
                    // so we're going to hardcode it, but later we'll need a way to dynamically/progamatically decide what interfaces to bind
                    // it is also going to be determined through wifi-direct routing or hotsport routing when these features are built
                    // Or if not, the network service will find a suitable Ip address to bind to, maybe that one in an active Wifi network
                    // Or if not, we can just leave binding to all interface (0.0.0.0) to ensure any kind of packet meant to reach this core is delivered no matter what interface gave it
                    // it could be a wifi, hotsport, or a VPN networks, or mobile networks but for the ip_addr field in the snapshot, We find it dynamically/progamatically the code should figure out what is the active Interface by now
                    // and this is best design than binding to a specific addr
                    ip_addr: "0.0.0.0".into()
                });
            }
        }
    }
}