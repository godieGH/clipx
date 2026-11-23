use super::*;

/// Ordering used when listing paired devices: connecting first, unreachable last.
pub(super) fn connection_rank(state: i32) -> u8 {
    match proto::ConnectionState::try_from(state).unwrap_or(proto::ConnectionState::Disconnected) {
        proto::ConnectionState::Connecting => 0,
        proto::ConnectionState::Connected => 1,
        proto::ConnectionState::Disconnected => 2,
        proto::ConnectionState::Unavailable => 3,
    }
}

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    /// Trusted devices enriched with live address/port from `seen` and live
    /// connection state.
    pub(super) async fn build_paired_devices(&self) -> Vec<proto::DeviceInfo> {
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

    pub(super) async fn handle_device_command(&mut self, cmd: DeviceCommands) {
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
