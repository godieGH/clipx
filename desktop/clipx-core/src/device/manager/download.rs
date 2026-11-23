use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn on_file_download_request(
        &mut self,
        device_id: String,
        req: proto::FileDownloadRequest,
    ) {
        let _ = self
            .clipboard_tx
            .send(ClipboardCommand::PeerFileDownloadRequest {
                device_id,
                file_id: req.file_id,
                offset: req.offset,
            });
    }

    pub(super) fn on_file_chunk(&mut self, device_id: String, chunk: proto::FileChunk) {
        let handler_start = Instant::now();
        tracing::trace!(
            device_id = %shorter!(&device_id),
            file_id = %chunk.file_id,
            seq = chunk.seq,
            offset = chunk.offset,
            bytes = chunk.data.len(),
            eof = chunk.eof,
            "file download chunk received"
        );

        let Some(pending) = self.pending_downloads.get(&chunk.file_id).cloned() else {
            return;
        };
        if pending.device_id != device_id || pending.received_complete {
            return;
        }
        // A chunk we already have (a restarted or replaced sender stream
        // re-sending data from before our confirmed offset) is harmless. Drop it
        // quietly rather than aborting the whole download.
        if chunk.total_size == pending.total_size
            && !chunk.data.is_empty()
            && chunk.data.len() <= MAX_FILE_CHUNK_BYTES
            && chunk.offset.saturating_add(chunk.data.len() as u64) <= pending.received_size
        {
            tracing::debug!(file_id = %chunk.file_id, offset = chunk.offset, "ignoring duplicate file chunk");
            return;
        }
        if chunk.total_size != pending.total_size
            || chunk.data.len() > MAX_FILE_CHUNK_BYTES
            || chunk.offset != pending.received_size
            || chunk.offset.saturating_add(chunk.data.len() as u64) > pending.total_size
        {
            self.abort_download(&chunk.file_id, "Invalid file chunk metadata");
            return;
        }
        let t = Instant::now();
        let digest = Sha256::digest(&chunk.data);
        let hash_time = t.elapsed();
        if chunk.sha256.len() != digest.len() || chunk.sha256.as_slice() != digest.as_slice() {
            self.abort_download(&chunk.file_id, "File chunk integrity check failed");
            return;
        }
        let key = (device_id.clone(), chunk.file_id.clone());
        let temp_path = self.incoming_temp_path(&device_id, &chunk.file_id);
        let state_path = self.incoming_state_path(&chunk.file_id);
        if !self.incoming_files.contains_key(&key) {
            if fs::create_dir_all(&self.incoming_files_dir).is_err() {
                self.abort_download(&chunk.file_id, "Could not prepare incoming storage");
                return;
            }
            let file = if pending.received_size == 0 {
                match OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(&temp_path)
                {
                    Ok(f) => f,
                    Err(_) => {
                        let _ = fs::remove_file(&temp_path);
                        match OpenOptions::new()
                            .create_new(true)
                            .read(true)
                            .write(true)
                            .open(&temp_path)
                        {
                            Ok(f) => f,
                            Err(_) => {
                                self.abort_download(
                                    &chunk.file_id,
                                    "Could not create incoming file",
                                );
                                return;
                            }
                        }
                    }
                }
            } else {
                let Ok(meta) = fs::metadata(&temp_path) else {
                    self.abort_download(&chunk.file_id, "Resume data is missing");
                    return;
                };
                if meta.len() != pending.received_size {
                    self.abort_download(
                        &chunk.file_id,
                        "Resume data does not match the confirmed offset",
                    );
                    return;
                }
                match OpenOptions::new().read(true).write(true).open(&temp_path) {
                    Ok(f) => f,
                    Err(_) => {
                        self.abort_download(&chunk.file_id, "Could not reopen incoming file");
                        return;
                    }
                }
            };
            self.incoming_files.insert(
                key.clone(),
                IncomingFileAssembly {
                    next_seq: pending.received_size / MAX_FILE_CHUNK_BYTES as u64,
                    total_size: pending.total_size,
                    received_size: pending.received_size,
                    temp_path: temp_path.clone(),
                    state_path: state_path.clone(),
                    file,
                    profile: ReceiverProfile::new(),
                },
            );
        }
        let new_offset = {
            let Some(state) = self.incoming_files.get_mut(&key) else {
                self.abort_download(&chunk.file_id, "Incoming file session is missing");
                return;
            };
            if state.next_seq != chunk.seq || state.received_size != chunk.offset {
                self.abort_download(&chunk.file_id, "Invalid file chunk sequence");
                return;
            }

            let t = Instant::now();
            let mut ok = state
                .file
                .seek(SeekFrom::Start(state.received_size))
                .is_ok();
            state.profile.seek.record(t.elapsed());

            if ok {
                let t = Instant::now();
                ok = state.file.write_all(&chunk.data).is_ok();
                state.profile.write.record(t.elapsed());
            }
            if ok {
                let t = Instant::now();
                ok = state.file.sync_all().is_ok();
                state.profile.sync.record(t.elapsed());
            }
            if !ok {
                self.abort_download(&chunk.file_id, "Could not persist incoming file data");
                return;
            }
            state.received_size.saturating_add(chunk.data.len() as u64)
        };
        if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
            active.received_size = new_offset;
            active.last_activity = Instant::now();
            active.stall_retries = 0;
        }
        let mut resume_time = Duration::ZERO;
        if let Some(active) = self.pending_downloads.get(&chunk.file_id) {
            let t = Instant::now();
            self.persist_resume_state(&device_id, &chunk.file_id, active, new_offset);
            resume_time = t.elapsed();
        }
        let t = Instant::now();
        self.send_peer(
            &device_id,
            Body::FileChunkAck(proto::FileChunkAck {
                file_id: chunk.file_id.clone(),
                confirmed_offset: new_offset,
            }),
        );
        let ack_time = t.elapsed();

        if let Some(state) = self.incoming_files.get_mut(&key) {
            let p = &mut state.profile;
            p.hash.record(hash_time);
            p.resume_state.record(resume_time);
            p.ack_queue.record(ack_time);
            p.handler.record(handler_start.elapsed());
            p.bytes += chunk.data.len() as u64;
        }
        tracing::trace!(
            device_id = %shorter!(&device_id),
            file_id = %chunk.file_id,
            seq = chunk.seq,
            offset = chunk.offset,
            bytes = chunk.data.len(),
            hash_us = hash_time.as_micros() as u64,
            resume_state_us = resume_time.as_micros() as u64,
            ack_enqueue_us = ack_time.as_micros() as u64,
            "file download chunk processed"
        );
        if chunk.eof {
            if new_offset != pending.total_size {
                self.abort_download(&chunk.file_id, "File ended before all data arrived");
                return;
            }
            let Some(done_state) = self.incoming_files.remove(&key) else {
                return;
            };
            done_state.profile.log_summary(&chunk.file_id, "complete");
            let _ = fs::remove_file(&done_state.state_path);
            if let Some(active) = self.pending_downloads.get_mut(&chunk.file_id) {
                active.received_complete = true;
            }
            self.notify_transfer(
                &pending.device_id,
                &pending.entry_id,
                &chunk.file_id,
                &pending.file_name,
                "download",
                new_offset,
                pending.total_size,
                "receiving",
                format!("Receiving {}", pending.file_name),
            );
            if self
                .clipboard_tx
                .send(ClipboardCommand::IncomingFilePath {
                    source_device_id: device_id,
                    file_id: chunk.file_id.clone(),
                    path: done_state.temp_path.clone(),
                })
                .is_err()
            {
                let _ = fs::remove_file(done_state.temp_path);
                self.release_pending_download(&chunk.file_id);
                self.notify_transfer(
                    &pending.device_id,
                    &pending.entry_id,
                    &chunk.file_id,
                    &pending.file_name,
                    "download",
                    new_offset,
                    pending.total_size,
                    "failed",
                    "Clipboard manager is unavailable",
                );
            }
            return;
        }
        if let Some(state) = self.incoming_files.get_mut(&key) {
            state.next_seq = state.next_seq.saturating_add(1);
            state.received_size = new_offset;
        }
        self.notify_transfer(
            &pending.device_id,
            &pending.entry_id,
            &chunk.file_id,
            &pending.file_name,
            "download",
            new_offset,
            pending.total_size,
            "receiving",
            format!(
                "Downloading {} · {}%",
                pending.file_name,
                Self::percentage(new_offset, pending.total_size)
            ),
        );
    }

    /// The sender asked us to continue a transfer we hold a partial download
    /// of. We ask again from the kept offset.
    pub(super) fn on_file_transfer_resume(
        &mut self,
        device_id: String,
        resume: proto::FileTransferResume,
    ) {
        let file_id = resume.file_id;
        if self.pending_downloads.contains_key(&file_id) || !self.connected.contains_key(&device_id)
        {
            return;
        }
        let Some(state) = fs::read(self.incoming_state_path(&file_id))
            .ok()
            .and_then(|b| serde_json::from_slice::<ResumeState>(&b).ok())
        else {
            return;
        };
        if state.device_id != device_id || state.offer_expires_at_ms < Self::now_ms() {
            return;
        }
        self.resume_candidates.remove(&file_id);
        self.start_or_resume_download(
            state.device_id,
            state.file_id,
            state.entry_id,
            state.file_name,
            state.total_size,
            state.offer_expires_at_ms,
        );
    }

    /// Drop a kept partial download the sender cancelled, and report it so the
    /// row does not stay "paused" for a transfer that can no longer continue.
    pub(super) fn discard_kept_partial(&mut self, device_id: &str, file_id: &str) {
        let Some(state) = fs::read(self.incoming_state_path(file_id))
            .ok()
            .and_then(|b| serde_json::from_slice::<ResumeState>(&b).ok())
        else {
            return;
        };
        if state.device_id != device_id {
            return;
        }
        let _ = fs::remove_file(self.incoming_temp_path(device_id, file_id));
        let _ = fs::remove_file(self.incoming_state_path(file_id));
        let _ = fs::remove_file(self.incoming_paused_path(file_id));
        self.resume_candidates.remove(file_id);
        self.notify_transfer(
            device_id,
            &state.entry_id,
            file_id,
            &state.file_name,
            "download",
            state.confirmed_offset,
            state.total_size,
            "cancelled",
            format!("The sender cancelled {}", state.file_name),
        );
    }

    pub(super) fn start_or_resume_download(
        &mut self,
        device_id: String,
        file_id: String,
        entry_id: String,
        file_name: String,
        total_size: u64,
        offer_expires_at_ms: u64,
    ) {
        if self.pending_downloads.contains_key(&file_id) || !self.connected.contains_key(&device_id)
        {
            return;
        }
        let _ = fs::remove_file(self.incoming_paused_path(&file_id));
        let offset = self
            .load_valid_resume_state(
                &file_id,
                &device_id,
                &entry_id,
                &file_name,
                total_size,
                offer_expires_at_ms,
            )
            .map(|s| s.confirmed_offset)
            .unwrap_or(0);
        if offset == 0 {
            let _ = fs::remove_file(self.incoming_temp_path(&device_id, &file_id));
            let _ = fs::remove_file(self.incoming_state_path(&file_id));
        }
        self.pending_downloads.insert(
            file_id.clone(),
            PendingDownload {
                device_id: device_id.clone(),
                entry_id: entry_id.clone(),
                file_name: file_name.clone(),
                total_size,
                offer_expires_at_ms,
                received_size: offset,
                received_complete: false,
                last_activity: Instant::now(),
                stall_retries: 0,
            },
        );
        self.notify_transfer(
            &device_id,
            &entry_id,
            &file_id,
            &file_name,
            "download",
            offset,
            total_size,
            "requesting",
            if offset > 0 {
                format!(
                    "Resuming {file_name} · {}%",
                    Self::percentage(offset, total_size)
                )
            } else {
                "Downloading file".into()
            },
        );
        self.send_peer(
            &device_id,
            Body::FileDownloadRequest(proto::FileDownloadRequest { file_id, offset }),
        );
    }

    /// Watchdog for incoming transfers. Runs on the manager's 10s tick.
    ///
    /// If a download is still wanted and the peer is connected but no chunk has
    /// arrived for DOWNLOAD_STALL_TIMEOUT, ask the sender to continue from our
    /// current offset over the same connection. After DOWNLOAD_MAX_STALL_RETRIES
    /// unanswered attempts, report the transfer as interrupted; it still resumes
    /// on the next reconnect (`resume_downloads_for_device`) or if chunks resume.
    pub(super) fn check_stalled_downloads(&mut self) {
        let stalled: Vec<String> = self
            .pending_downloads
            .iter()
            .filter(|(_, p)| {
                !p.received_complete
                    && p.stall_retries <= DOWNLOAD_MAX_STALL_RETRIES
                    && self.connected.contains_key(&p.device_id)
                    && p.last_activity.elapsed() >= DOWNLOAD_STALL_TIMEOUT
            })
            .map(|(file_id, _)| file_id.clone())
            .collect();
        for file_id in stalled {
            let Some(p) = self.pending_downloads.get_mut(&file_id) else {
                continue;
            };
            let give_up = p.stall_retries >= DOWNLOAD_MAX_STALL_RETRIES;
            p.last_activity = Instant::now();
            p.stall_retries = p.stall_retries.saturating_add(1);
            let snapshot = p.clone();
            if give_up {
                tracing::warn!(%file_id, "download stalled; giving up until retried");
                self.notify_transfer(
                    &snapshot.device_id,
                    &snapshot.entry_id,
                    &file_id,
                    &snapshot.file_name,
                    "download",
                    snapshot.received_size,
                    snapshot.total_size,
                    "interrupted",
                    "The sender stopped responding; the download can resume later",
                );
            } else {
                tracing::info!(
                    %file_id,
                    attempt = snapshot.stall_retries,
                    offset = snapshot.received_size,
                    "download stalled; re-requesting from current offset"
                );
                self.notify_transfer(
                    &snapshot.device_id,
                    &snapshot.entry_id,
                    &file_id,
                    &snapshot.file_name,
                    "download",
                    snapshot.received_size,
                    snapshot.total_size,
                    "requesting",
                    format!(
                        "Resuming {} · {}%",
                        snapshot.file_name,
                        Self::percentage(snapshot.received_size, snapshot.total_size)
                    ),
                );
                self.send_peer(
                    &snapshot.device_id,
                    Body::FileDownloadRequest(proto::FileDownloadRequest {
                        file_id: file_id.clone(),
                        offset: snapshot.received_size,
                    }),
                );
            }
        }
    }

    pub(super) fn resume_downloads_for_device(&mut self, device_id: &str) {
        let pending: Vec<(String, PendingDownload)> = self
            .pending_downloads
            .iter()
            .filter(|(_, p)| p.device_id == device_id && !p.received_complete)
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect();
        for (file_id, p) in pending {
            self.send_peer(
                &p.device_id,
                Body::FileDownloadRequest(proto::FileDownloadRequest {
                    file_id,
                    offset: p.received_size,
                }),
            );
        }
        let candidates: Vec<ResumeState> = self
            .resume_candidates
            .values()
            .filter(|s| s.device_id == device_id && s.offer_expires_at_ms >= Self::now_ms())
            .cloned()
            .collect();
        for state in candidates {
            self.resume_candidates.remove(&state.file_id);
            self.start_or_resume_download(
                state.device_id,
                state.file_id,
                state.entry_id,
                state.file_name,
                state.total_size,
                state.offer_expires_at_ms,
            );
        }
    }

    pub(super) fn abort_download(&mut self, file_id: &str, message: impl Into<String>) {
        let message = message.into();
        tracing::warn!(%file_id, error = %message, "file download aborted");
        let pending = self.pending_downloads.remove(file_id);
        if let Some(p) = pending {
            if let Some(s) = self
                .incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
            {
                s.profile.log_summary(file_id, "aborted");
                let _ = fs::remove_file(s.temp_path);
            }
            self.resume_candidates.remove(file_id);
            let _ = fs::remove_file(self.incoming_state_path(file_id));
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
            self.notify_transfer(
                &p.device_id,
                &p.entry_id,
                file_id,
                &p.file_name,
                "download",
                p.received_size,
                p.total_size,
                "failed",
                message,
            );
        }
    }

    pub(super) fn cancel_local_download(&mut self, file_id: &str) {
        if let Some(p) = self.pending_downloads.remove(file_id)
            && let Some(s) = self
                .incoming_files
                .remove(&(p.device_id.clone(), file_id.to_string()))
        {
            let _ = fs::remove_file(s.temp_path);
        }
        self.resume_candidates.remove(file_id);
        let _ = fs::remove_file(self.incoming_state_path(file_id));
        let keys: Vec<(String, String)> = self
            .incoming_files
            .keys()
            .filter(|(_, id)| id == file_id)
            .cloned()
            .collect();
        for k in keys {
            if let Some(s) = self.incoming_files.remove(&k) {
                let _ = fs::remove_file(s.temp_path);
            }
        }
        let _ = self
            .clipboard_tx
            .send(ClipboardCommand::FileDownloadReleased {
                file_id: file_id.to_string(),
            });
    }

    pub(super) fn release_pending_download(&mut self, file_id: &str) -> Option<PendingDownload> {
        let pending = self.pending_downloads.remove(file_id);
        if pending.is_some() {
            let _ = self
                .clipboard_tx
                .send(ClipboardCommand::FileDownloadReleased {
                    file_id: file_id.to_string(),
                });
        }
        pending
    }

    /// Retire a download that stopped for a reason other than failure.
    /// `keep_partial` leaves the temp file and resume state on disk so the
    /// download can continue from the confirmed offset; otherwise both go.
    pub(super) fn park_download(
        &mut self,
        p: &PendingDownload,
        file_id: &str,
        keep_partial: bool,
        state: &str,
        message: String,
    ) {
        if let Some(s) = self
            .incoming_files
            .remove(&(p.device_id.clone(), file_id.to_string()))
        {
            s.profile.log_summary(file_id, state);
            if !keep_partial {
                let _ = fs::remove_file(&s.temp_path);
            }
        }
        // Never leave it on the auto-resume list: only an explicit request
        // (or, for an interruption, a reconnect scan on next start) resumes it.
        self.resume_candidates.remove(file_id);
        if !keep_partial {
            let _ = fs::remove_file(self.incoming_temp_path(&p.device_id, file_id));
            let _ = fs::remove_file(self.incoming_state_path(file_id));
            let _ = fs::remove_file(self.incoming_paused_path(file_id));
        }
        self.notify_transfer(
            &p.device_id,
            &p.entry_id,
            file_id,
            &p.file_name,
            "download",
            p.received_size,
            p.total_size,
            state,
            message,
        );
    }
}
