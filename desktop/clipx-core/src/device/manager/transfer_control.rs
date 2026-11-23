use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn peer_name(&self, device_id: &str) -> String {
        self.trusted
            .list()
            .find(|d| d.id == device_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| device_id.to_string())
    }

    /// Whether this peer advertised pause/resume in its hello. Older builds
    /// (and peers that sent no hello at all) do not, and still transfer fine.
    pub(super) fn peer_supports_transfer_control(&self, device_id: &str) -> bool {
        self.effective_versions
            .get(device_id)
            .is_some_and(|(_, caps)| {
                caps.contains(&(proto::PeerCapabilities::CapabilityFileTransferControl as i32))
            })
    }

    /// A user's pause / resume / cancel on one transfer. Cancel is always
    /// allowed (every build understands FileTransferCancel). Pause and resume
    /// need the peer's cooperation, so they are refused, with an explanation
    /// for the toast, when the peer is an older build.
    pub(super) fn control_transfer(
        &mut self,
        file_id: &str,
        device_id: Option<&str>,
        action: TransferAction,
    ) -> Result<String, String> {
        let wanted = |d: &str| device_id.is_none_or(|w| w == d);
        let mut running: Vec<String> = Vec::new();
        if let Some(p) = self.pending_downloads.get(file_id)
            && wanted(&p.device_id)
        {
            running.push(p.device_id.clone());
        }
        running.extend(
            self.upload_sessions
                .keys()
                .filter(|(d, f)| f == file_id && wanted(d))
                .map(|(d, _)| d.clone()),
        );
        let held: Vec<String> = self
            .held_uploads
            .keys()
            .filter(|(d, f)| f == file_id && wanted(d))
            .map(|(d, _)| d.clone())
            .collect();
        let paused: Vec<ResumeState> = self
            .paused_downloads()
            .into_iter()
            .filter(|s| s.file_id == file_id && wanted(&s.device_id))
            .collect();
        let refusal = |this: &Self, peers: &[String]| -> Option<String> {
            peers
                .iter()
                .find(|d| !this.peer_supports_transfer_control(d))
                .map(|d| {
                    format!(
                        "{} runs an older ClipX that can't pause or resume transfers. You can still cancel.",
                        this.peer_name(d)
                    )
                })
        };
        match action {
            TransferAction::Cancel => {
                if running.is_empty() && held.is_empty() && paused.is_empty() {
                    return Err("No running transfer to cancel".into());
                }
                self.stop_transfer(file_id, device_id, false);
                // A paused download has nothing running: drop its kept partial
                // data and tell the sender it will not continue.
                for state in paused {
                    self.discard_kept_partial(&state.device_id, file_id);
                    self.send_peer(
                        &state.device_id,
                        Body::FileTransferCancel(proto::FileTransferCancel {
                            file_id: file_id.to_string(),
                            pause: false,
                        }),
                    );
                }
                Ok("transfer cancelled".into())
            }
            TransferAction::Pause => {
                if running.is_empty() {
                    return Err("No running transfer to pause".into());
                }
                if let Some(message) = refusal(self, &running) {
                    return Err(message);
                }
                self.stop_transfer(file_id, device_id, true);
                Ok("transfer paused".into())
            }
            TransferAction::Resume => {
                // Only a paused upload is resumed from here; a paused download
                // is resumed by asking for the file again.
                if held.is_empty() {
                    return Err(
                        "Nothing to resume here: the device receiving the file resumes it".into(),
                    );
                }
                if let Some(message) = refusal(self, &held) {
                    return Err(message);
                }
                for peer in held {
                    let key = (peer.clone(), file_id.to_string());
                    let Some(h) = self.held_uploads.get_mut(&key) else {
                        continue;
                    };
                    h.waiting = true;
                    let (name, done, total) = (h.file_name.clone(), h.done, h.total_size);
                    self.send_peer(
                        &peer,
                        Body::FileTransferResume(proto::FileTransferResume {
                            file_id: file_id.to_string(),
                        }),
                    );
                    let waiting_for = self.peer_name(&peer);
                    self.notify_transfer(
                        &peer,
                        file_id,
                        file_id,
                        &name,
                        "send",
                        done,
                        total,
                        "requesting",
                        format!("Waiting for {waiting_for} to continue"),
                    );
                }
                Ok("waiting for the receiver to continue".into())
            }
        }
    }

    /// Stop transfers of `file_id`, optionally only the one with `device_id`.
    /// The history entry and the sender's staged offer are left alone, so the
    /// file can be downloaded again. `pause` keeps the partial download
    /// (continued from the confirmed offset on the next request); otherwise
    /// it is discarded.
    pub(super) fn stop_transfer(&mut self, file_id: &str, device_id: Option<&str>, pause: bool) {
        let wanted = |d: &str| device_id.is_none_or(|w| w == d);
        // Receiving side.
        if let Some(p) = self
            .pending_downloads
            .get(file_id)
            .filter(|p| wanted(&p.device_id))
            .cloned()
        {
            self.pending_downloads.remove(file_id);
            // Free the clipboard manager's slot before replying, so a Resume
            // right after cannot be refused as "already in progress".
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
            if pause {
                let _ = fs::write(
                    self.incoming_paused_path(file_id),
                    Self::now_ms().to_string(),
                );
            }
            let (state, message) = if pause {
                ("paused", format!("Paused {}", p.file_name))
            } else {
                ("cancelled", format!("Cancelled {}", p.file_name))
            };
            self.park_download(&p, file_id, pause, state, message);
            self.send_peer(
                &p.device_id,
                Body::FileTransferCancel(proto::FileTransferCancel {
                    file_id: file_id.to_string(),
                    pause,
                }),
            );
        }
        // Sending side: every session streaming this file to a wanted device.
        let keys: Vec<(String, String)> = self
            .upload_sessions
            .keys()
            .filter(|(d, f)| f == file_id && wanted(d))
            .cloned()
            .collect();
        for key in keys {
            if let Some(s) = self.upload_sessions.remove(&key) {
                if pause {
                    s.paused.store(true, Ordering::Release);
                    self.held_uploads.insert(
                        key.clone(),
                        HeldUpload {
                            file_name: s.file_name.clone(),
                            total_size: s.total_size,
                            done: s.confirmed.load(Ordering::Acquire),
                            waiting: false,
                        },
                    );
                    s.interrupt.cancel();
                } else {
                    s.cancel.cancel();
                }
            }
            self.send_peer(
                &key.0,
                Body::FileTransferCancel(proto::FileTransferCancel {
                    file_id: file_id.to_string(),
                    pause,
                }),
            );
        }
        // A paused (or waiting) upload has no task left to report a cancel.
        if !pause {
            let held: Vec<(String, String)> = self
                .held_uploads
                .keys()
                .filter(|(d, f)| f == file_id && wanted(d))
                .cloned()
                .collect();
            for key in held {
                if let Some(h) = self.held_uploads.remove(&key) {
                    self.notify_transfer(
                        &key.0,
                        file_id,
                        file_id,
                        &h.file_name,
                        "send",
                        h.done,
                        h.total_size,
                        "cancelled",
                        format!("Sending {} was cancelled", h.file_name),
                    );
                    self.send_peer(
                        &key.0,
                        Body::FileTransferCancel(proto::FileTransferCancel {
                            file_id: file_id.to_string(),
                            pause: false,
                        }),
                    );
                }
            }
        }
    }

    /// Downloads the user paused: partial data and resume state are on disk,
    /// nothing is running. They survive an app restart.
    pub(super) fn paused_downloads(&self) -> Vec<ResumeState> {
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("resume") {
                    return None;
                }
                let state: ResumeState = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                (self.incoming_paused_path(&state.file_id).exists()
                    && !self.pending_downloads.contains_key(&state.file_id)
                    && state.offer_expires_at_ms >= Self::now_ms())
                .then_some(state)
            })
            .collect()
    }

    /// Every live transfer, for a UI that opened after they started.
    pub(super) fn list_transfers(&self) -> Vec<TransferSnapshot> {
        let mut out = Vec::new();
        for (file_id, p) in &self.pending_downloads {
            out.push(TransferSnapshot {
                device_id: p.device_id.clone(),
                device_name: self.peer_name(&p.device_id),
                entry_id: p.entry_id.clone(),
                file_id: file_id.clone(),
                file_name: p.file_name.clone(),
                direction: "download".into(),
                done: p.received_size,
                total: p.total_size,
                state: if p.received_size > 0 {
                    "receiving"
                } else {
                    "requesting"
                }
                .into(),
                can_pause: self.peer_supports_transfer_control(&p.device_id),
            });
        }
        for state in self.paused_downloads() {
            out.push(TransferSnapshot {
                device_id: state.device_id.clone(),
                device_name: self.peer_name(&state.device_id),
                entry_id: state.entry_id.clone(),
                file_id: state.file_id.clone(),
                file_name: state.file_name.clone(),
                direction: "download".into(),
                done: state.confirmed_offset,
                total: state.total_size,
                state: "paused".into(),
                can_pause: self.peer_supports_transfer_control(&state.device_id),
            });
        }
        for ((device_id, file_id), s) in &self.upload_sessions {
            out.push(TransferSnapshot {
                device_id: device_id.clone(),
                device_name: self.peer_name(device_id),
                entry_id: file_id.clone(),
                file_id: file_id.clone(),
                file_name: s.file_name.clone(),
                direction: "send".into(),
                done: s.confirmed.load(Ordering::Acquire),
                total: s.total_size,
                state: "sending".into(),
                can_pause: self.peer_supports_transfer_control(device_id),
            });
        }
        for ((device_id, file_id), h) in &self.held_uploads {
            out.push(TransferSnapshot {
                device_id: device_id.clone(),
                device_name: self.peer_name(device_id),
                entry_id: file_id.clone(),
                file_id: file_id.clone(),
                file_name: h.file_name.clone(),
                direction: "send".into(),
                done: h.done,
                total: h.total_size,
                state: if h.waiting { "requesting" } else { "paused" }.into(),
                can_pause: self.peer_supports_transfer_control(device_id),
            });
        }
        out
    }

    pub(super) fn notify_transfer(
        &mut self,
        device_id: &str,
        entry_id: &str,
        file_id: &str,
        file_name: &str,
        direction: &str,
        done: u64,
        total: u64,
        state: &str,
        message: impl Into<String>,
    ) {
        let message = message.into();
        let terminal = matches!(
            state,
            "complete" | "failed" | "expired" | "cancelled" | "interrupted" | "paused"
        );
        let should_emit = if terminal || total == 0 || !matches!(state, "receiving" | "sending") {
            true
        } else {
            let percent = ((done.saturating_mul(100)) / total).min(100) as u8;
            let key = (state.to_string(), percent);
            if self.transfer_progress.get(file_id) == Some(&key) {
                false
            } else {
                self.transfer_progress.insert(file_id.to_string(), key);
                true
            }
        };
        if !should_emit {
            return;
        }
        if terminal {
            tracing::debug!(%file_id, direction, state, done, total, "file transfer state changed");
        } else {
            tracing::trace!(%file_id, direction, state, done, total, "file transfer progress emitted");
        }
        if terminal || state == "interrupted" || state == "cancelled" {
            self.transfer_progress.remove(file_id);
        }
        let _ = self.events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: entry_id.to_string(),
            file_id: file_id.to_string(),
            file_name: file_name.to_string(),
            direction: direction.to_string(),
            done,
            total,
            state: state.to_string(),
            message: message.clone(),
            device_id: device_id.to_string(),
            device_name: self.peer_name(device_id),
            can_pause: self.peer_supports_transfer_control(device_id),
        });
        let notification_future = self.notification.notify_file_transfer(
            file_id.to_string(),
            file_name.to_string(),
            direction.to_string(),
            done,
            total,
            state.to_string(),
            message,
            None,
        );
        // The native notification event is enqueued synchronously above.
        // Only the acknowledgment wait is spawned, so a later transfer state
        // cannot overtake an earlier one merely because Tokio scheduled its
        // wrapper task first.
        tokio::spawn(notification_future);
    }

    pub(super) async fn emit_transfer(
        events_tx: &broadcast::Sender<CoreEvent>,
        notification: &E,
        device_id: &str,
        device_name: &str,
        can_pause: bool,
        file_id: &str,
        file_name: &str,
        direction: &str,
        done: u64,
        total: u64,
        state: &'static str,
        message: String,
    ) {
        let _ = events_tx.send(CoreEvent::FileTransferChanged {
            entry_id: file_id.to_string(),
            file_id: file_id.to_string(),
            file_name: file_name.to_string(),
            direction: direction.to_string(),
            done,
            total,
            state: state.to_string(),
            message: message.clone(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            can_pause,
        });
        notification
            .notify_file_transfer(
                file_id.to_string(),
                file_name.to_string(),
                direction.to_string(),
                done,
                total,
                state.to_string(),
                message,
                None,
            )
            .await;
    }

    pub(super) fn percentage(done: u64, total: u64) -> u8 {
        done.saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u8
    }
}
