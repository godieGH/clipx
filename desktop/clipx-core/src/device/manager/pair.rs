use super::*;

pub(super) fn get_formatted_fp(device_id: &str) -> String {
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
    // ---------------- Pair: initiator side ----------------

    pub(super) fn handle_pair_request(&mut self, device_id: &str) -> String {
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
        tracing::debug!(device_id = %shorter!(device_id), "pairing initiated");
        self.notify_pair_events(
            device_id,
            proto::pairing_event::State::Started as i32,
            "pairing started",
        );
        self.dial(device_id, addr);
        "pairing started".to_string()
    }

    pub(super) fn reject_pair_request(
        &self,
        device_id: &str,
        connection_id: &str,
        code: u32,
        message: &str,
    ) {
        self.send_control_on_connection(device_id, connection_id, code, message);

        self.request_transport_disconnect_on_connection(device_id, connection_id);
    }

    // ---------------- Pair: responder side ----------------

    pub(super) fn on_pair_request(
        &mut self,
        device_id: String,
        connection_id: String,
        req: proto::PeerPairRequest,
        notify_tx: &mpsc::UnboundedSender<NotifyResult>,
    ) {
        // Legacy peers send no hello: let them through, gate future features by capabilities.
        if let Err(msg) = self.check_hello(&device_id, req.hello.clone()) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
            return;
        }

        if self.trusted.is_trusted(&device_id) {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::ALREADY_TRUSTED,
                "already trusted",
            );
            return;
        }
        let Ok(pubkey): Result<[u8; 32], _> = req.public_key.as_slice().try_into() else {
            self.reject_pair_request(
                &device_id,
                &connection_id,
                pairing::control::PROTOCOL_ERROR,
                "malformed public key",
            );
            return;
        };
        if hex::encode(DeviceIdentity::get_fingerprint_for(pubkey)) != device_id {
            self.reject_pair_request(
                &device_id,
                &connection_id,
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

    pub(super) fn on_pair_challenge(
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

    pub(super) fn on_pair_ack(&mut self, device_id: String) {
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

    pub(super) fn on_pair_response(
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
        if let Err(msg) = self.check_hello(&device_id, res.hello.clone()) {
            self.abort_pair(
                &device_id,
                pairing::control::PROTOCOL_VERSION_COMPATIBILITY_ERROR,
                &msg,
            );
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

    pub(super) fn on_pair_challenge_response(
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
    pub(super) fn finalize_initiator_pair(&mut self, device_id: &str) {
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
                pairing::control::INVALID_SIGNATURE,
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

    pub(super) fn finalize_pair_trusted(&mut self, device_id: &str) {
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

    pub(super) fn handle_notify_result(&mut self, result: NotifyResult) {
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

    pub(super) fn on_pair_approval_result(&mut self, device_id: String, decision: PairDecision) {
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
                        hello: Some(config::local_hello()),
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

    pub(super) fn on_code_confirm_result(&mut self, device_id: String, decision: PairDecision) {
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

    pub(super) fn abort_pair(&mut self, device_id: &str, code: u32, message: &str) {
        self.send_control(device_id, code, message);
        self.pair_sessions.remove(device_id);
        self.notify_pair_events(
            device_id,
            proto::pairing_event::State::Failed as i32,
            message,
        );
        self.request_transport_disconnect(device_id);
    }

    pub(super) fn list_pending_pairings(&self) -> Vec<PendingPairEntry> {
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
    pub(super) fn handle_approve_pairing(&mut self, device_id: &str, approve: bool) -> String {
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
