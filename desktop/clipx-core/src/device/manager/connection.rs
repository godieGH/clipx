use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn handle_announce(&mut self, announce: proto::Announce, addr: IpAddr) {
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
        } else {
            tracing::trace!(device_id = %shorter!(&device_id), %addr, ws_port = announce.ws_port, "peer discovery announcement refreshed");
        }

        self.seen.upsert(SeenDevice {
            id: device_id.clone(),
            name: announce.device_name.clone(),
            device_type,
            addr,
            ws_port: announce.ws_port,
            last_seen: Instant::now(),
        });

        if just_appeared {
            self.notify_devices_changed();
        }

        // Auto-connect is deliberately a *fresh discovery* trigger. Do not
        // reconnect merely because an already-visible peer disconnected, and
        // do not start a connection just because the user toggled this on.
        // Both sides are allowed to initiate here; the transport/handshake
        // resolves an overlap instead of using a fingerprint tie-breaker.
        if just_appeared
            && let Some(td) = self.trusted.get(&device_id)
            && td.auto_connect
            && !self.connected.contains_key(&device_id)
            && !self.connect_sessions.contains_key(&device_id)
        {
            let _ = self.handle_connect(&device_id);
        }
    }

    // ---------------- Connect: initiator side ----------------

    pub(super) fn handle_connect(&mut self, device_id: &str) -> String {
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
        tracing::debug!(device_id = %shorter!(device_id), %addr, "Handle connect: triggered/auto-connect");

        self.connect_sessions
            .insert(device_id.to_string(), ConnectSession::new_initiator(addr));

        // The peer may already have opened the transport while this logical
        // connect request was being created. Reuse that live connection rather
        // than opening another socket. This is still gated by the caller's
        // normal fresh-discovery/manual-connect rules.
        if let Some(connection_id) = self.transport_connections.get(device_id).cloned() {
            tracing::debug!(%connection_id, "Re-using an existing peer transport");
            self.on_transport_connected(device_id.to_string(), connection_id);
        } else {
            tracing::debug!(
                device_id = %shorter!(device_id),
                "Dialing new transport: no peer transport available"
            );
            self.dial(device_id, addr);
        }
        "connecting".to_string()
    }

    pub(super) fn dial(&self, device_id: &str, addr: SocketAddr) {
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

    pub(super) async fn handle_disconnect(&mut self, device_id: &str) -> String {
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
        self.transport_connections.remove(device_id);
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

    pub(super) fn handle_transport_event(
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
                profile,
            } => {
                if self
                    .connected
                    .get(&device_id)
                    .is_some_and(|current| current != &connection_id)
                {
                    return;
                }

                // Time spent waiting in the Transport -> DeviceManager channel.
                let event_queue_us = profile
                    .as_ref()
                    .map(|p| p.event_enqueued_at.elapsed().as_micros() as u64)
                    .unwrap_or(0);

                let device_id_for_log = device_id.clone();
                let connection_id_for_log = connection_id.clone();

                // Time spent processing the incoming message in DeviceManager.
                let started = Instant::now();

                self.on_peer_message(device_id, connection_id, message, notify_tx);

                let manager_dispatch_us = started.elapsed().as_micros() as u64;

                if let Some(profile) = profile {
                    tracing::trace!(
                        device_id = %device_id_for_log,
                        connection_id = %connection_id_for_log,
                        message_type = profile.trace.message_type,
                        file_id = %profile.trace.file_id,
                        confirmed_offset = profile.trace.confirmed_offset,
                        seq = ?profile.trace.seq,
                        payload_bytes = profile.trace.payload_bytes,
                        decode_us = profile.decode_us,
                        event_queue_us,
                        manager_dispatch_us,
                        "device manager inbound profile"
                    );
                }
            }
        }
    }

    pub(super) fn on_transport_connected(&mut self, device_id: String, connection_id: String) {
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
                        hello: Some(config::local_hello()),
                    }),
                );
            }
            return;
        }
        if let Some(sess) = self.connect_sessions.get_mut(&device_id)
            && sess.role == Role::Initiator
            && sess.stage == ConnectStage::Dialing
        {
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
                    hello: Some(config::local_hello()),
                }),
            );
        }
    }

    pub(super) fn on_transport_disconnected(&mut self, device_id: String, connection_id: String) {
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
            if let Some(s) = self
                .incoming_files
                .remove(&(device_id.clone(), file_id.clone()))
            {
                s.profile.log_summary(&file_id, "interrupted");
            }
            self.notify_transfer(
                &pending.device_id,
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
            tracing::info!("{device_id} no longer connected: connection {connection_id} closed");
            self.notify_devices_changed();
        }
        // drop the effective_version related to avoid stale
        self.effective_versions.remove(&device_id);
        // A paused upload has nothing left to resume once the receiver is gone.
        self.held_uploads.retain(|(d, _), _| d != &device_id);
    }

    /// A dial never became a live connection at all (TCP connect failed/timed
    /// out, or the WS handshake failed) — as opposed to on_transport_disconnected,
    /// which is for a connection that *was* live and then dropped. `dial()` is
    /// shared by both the pairing and connect initiator paths, so check both.
    pub(super) fn on_connect_failed(&mut self, device_id: String) {
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

    pub(super) fn on_peer_message(
        &mut self,
        device_id: String,
        connection_id: String,
        message: proto::PeerMessage,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        if let Some(body) = message.body.as_ref() {
            tracing::trace!(
                device_id = %shorter!(&device_id),
                %connection_id,
                message_kind = peer_message_name(body),
                "peer protocol message dispatched"
            );
        }
        if self
            .connected
            .get(&device_id)
            .is_some_and(|current| current != &connection_id)
        {
            return;
        }
        match message.body {
            Some(Body::PairRequest(req)) => {
                self.on_pair_request(device_id, connection_id, req, notify_tx)
            }
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
            Some(Body::FileTransferResume(resume)) => {
                self.on_file_transfer_resume(device_id, resume);
            }
            None => tracing::warn!("empty PeerMessage from {device_id}"),
        }
    }

    /// Version gate for a peer's hello. `Ok` records effective version + capabilities
    /// for this connection cycle; `Err(msg)` is the text to send to the peer.
    /// No hello (legacy peer) passes.
    pub(super) fn check_hello(
        &mut self,
        device_id: &str,
        hello: Option<Hello>,
    ) -> Result<(), String> {
        let Some(h) = hello else { return Ok(()) };
        match check_proto_version(h.proto_version, h.min_proto_version) {
            Gate::Ok { effective } => {
                self.effective_versions
                    .insert(device_id.to_string(), (effective, h.capabilities));
                Ok(())
            }
            Gate::PeerTooOld => {
                tracing::debug!(%device_id, "peer is too old, pair/connect failed");
                Err(format!(
                    "App is too old, upgrade to newer versions to pair/connect with: {}",
                    get_hostname()
                ))
            }
            Gate::SelfTooOld => {
                tracing::warn!(%device_id, "this app is too old to pair/connect to peer, upgrade");
                Err(format!(
                    "{} is too old to pair/connect, update it",
                    get_hostname()
                ))
            }
        }
    }

    // ---------------- Connect: responder side ----------------

    pub(super) fn on_connect_challenge(
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

        if let Err(msg) = self.check_hello(&device_id, c.hello.clone()) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
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
                hello: Some(config::local_hello()),
            }),
        );
    }

    pub(super) fn on_connect_ack(&mut self, device_id: String, connection_id: String) {
        let Some(sess) = self.connect_sessions.get(&device_id) else {
            tracing::warn!("ConnectAck from {device_id} ignored: no connect session");
            return;
        };
        if sess.role != Role::Responder
            || sess.stage != ConnectStage::AwaitingAck
            || sess.connection_id.as_deref() != Some(connection_id.as_str())
        {
            tracing::warn!(
                "ConnectAck from {device_id} ignored: role={:?} stage={:?} sess_conn={:?} ack_conn={}",
                sess.role,
                sess.stage,
                sess.connection_id,
                connection_id
            );
            return;
        }
        tracing::info!("connected to {device_id} on {connection_id}");
        self.connected.insert(device_id.clone(), connection_id);
        self.connect_sessions.remove(&device_id);
        self.resume_downloads_for_device(&device_id);
        self.notify_devices_changed();
    }

    // ---------------- Connect: initiator side (continued) ----------------

    pub(super) fn on_connect_challenge_response(
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
                pairing::control::INVALID_SIGNATURE,
                "signature verification failed",
            );
            return;
        }
        if let Err(msg) = self.check_hello(&device_id, r.hello.clone()) {
            self.abort_connect_on_connection(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }
        self.send_peer_on_connection(
            &device_id,
            &connection_id,
            Body::ConnectAck(proto::ConnectAck {}),
        );
        tracing::info!("connected to {device_id} on {connection_id}");
        self.connected.insert(device_id.clone(), connection_id);
        self.connect_sessions.remove(&device_id);
        self.resume_downloads_for_device(&device_id);
        self.notify_devices_changed();
    }

    pub(super) fn abort_connect(&mut self, device_id: &str, code: u32, message: &str) {
        self.send_control(device_id, code, message);
        self.connect_sessions.remove(device_id);
        self.request_transport_disconnect(device_id);
    }

    pub(super) fn abort_connect_on_connection(
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

    pub(super) fn on_control(
        &mut self,
        device_id: String,
        connection_id: String,
        ctrl: proto::PreTransportControl,
    ) {
        tracing::warn!(
            "pre-transport control from {device_id}: {} ({})",
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
            tracing::debug!(%device_id, ctrl_message=%&ctrl.message, "Pairing to peer failed, core emitting `proto::pairing_event::State::Failed` change");
            self.notify_pair_events(
                &device_id,
                proto::pairing_event::State::Failed as i32,
                &ctrl.message,
            );
        }
        self.notify_info("Pairing/connect failed", ctrl.message);
    }

    pub(super) fn prune_expired_sessions(&mut self) {
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
}
