use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn stream_file_to_peer(
        &mut self,
        device_id: String,
        file_id: String,
        path: PathBuf,
        total_size: u64,
        start_offset: u64,
    ) {
        let Some(transport_tx) = self.transport_tx.as_ref().cloned() else {
            return;
        };

        let key = (device_id.clone(), file_id.clone());

        let Some(connection_id) = self.connected.get(&device_id).cloned() else {
            return;
        };
        tracing::debug!(%device_id, %file_id, total_size, start_offset, "file upload session starting");

        // The receiver asked again while an older stream for the same file was
        // still registered (for example that task is bound to a socket that has
        // since been replaced). The newest request reflects the receiver's real
        // offset, so retire the old stream and start from the new one.
        if let Some(old) = self.upload_sessions.remove(&key) {
            old.superseded.store(true, Ordering::Release);
            old.interrupt.cancel();
        }
        self.next_upload_id = self.next_upload_id.wrapping_add(1);
        let session_id = self.next_upload_id;
        let superseded = Arc::new(AtomicBool::new(false));
        let superseded_task = superseded.clone();

        let cancel = CancellationToken::new();
        let interrupt = CancellationToken::new();
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let confirmed_offset = Arc::new(AtomicU64::new(start_offset));
        let confirmed_offset_task = confirmed_offset.clone();
        let paused_by_user = Arc::new(AtomicBool::new(false));
        let paused_task = paused_by_user.clone();
        // The receiver is asking again, so any paused/waiting upload is live.
        self.held_uploads.remove(&key);

        self.upload_sessions.insert(
            key.clone(),
            UploadSession {
                id: session_id,
                cancel: cancel.clone(),
                interrupt: interrupt.clone(),
                file_name: file_name.clone(),
                total_size,
                confirmed: confirmed_offset.clone(),
                paused: paused_by_user,
                superseded,
                ack_tx,
            },
        );
        let peer_id = device_id.clone();
        let peer_name = self.peer_name(&device_id);
        let can_pause = self.peer_supports_transfer_control(&device_id);

        let done_tx = self.upload_done_tx.clone();
        let events_tx = self.events_tx.clone();
        let notification = self.notification.clone();
        let connection_id_for_task = connection_id.clone();

        tokio::spawn(async move {
            let mut profile = SenderProfile::new();

            let result: Result<(), String> = async {
                let mut file = tokio::fs::File::open(&path)
                    .await
                    .map_err(|e| format!("Could not open file: {e}"))?;

                if file.metadata().await.map_err(|e| e.to_string())?.len() != total_size
                    || start_offset > total_size
                {
                    return Err("Source file changed or resume offset is invalid".into());
                }

                file.seek(SeekFrom::Start(start_offset))
                    .await
                    .map_err(|e| e.to_string())?;

                let mut offset = start_offset;
                let mut seq = offset / MAX_FILE_CHUNK_BYTES as u64;

                let _ = Self::emit_transfer(
                    &events_tx,
                    &notification,
                    &peer_id,
                    &peer_name,
                    can_pause,
                    &file_id,
                    &file_name,
                    "send",
                    offset,
                    total_size,
                    "sending",
                    "Sending file".into(),
                )
                .await;

                loop {
                    if interrupt.is_cancelled() {
                        return Err("INTERRUPTED".into());
                    }

                    if cancel.is_cancelled() {
                        return Err("CANCELLED".into());
                    }

                    let mut data = vec![0u8; MAX_FILE_CHUNK_BYTES];

                    let cycle_start = Instant::now();

                    let t = Instant::now();
                    let n = file.read(&mut data).await.map_err(|e| e.to_string())?;
                    profile.read.record(t.elapsed());

                    if n == 0 && offset != total_size {
                        return Err("Source file ended before the advertised size".into());
                    }

                    data.truncate(n);

                    let end = offset.saturating_add(n as u64);
                    let eof = end == total_size;
                    let t = Instant::now();
                    let digest = Sha256::digest(&data).to_vec();
                    profile.hash.record(t.elapsed());

                    // Re-check right before queueing so a retired stream does
                    // not slip one more chunk onto the connection.
                    if cancel.is_cancelled() {
                        return Err("CANCELLED".into());
                    }
                    if interrupt.is_cancelled() {
                        return Err("INTERRUPTED".into());
                    }

                    let t = Instant::now();
                    let (reply_tx, reply_rx) = oneshot::channel();

                    transport_tx
                        .send(TransportCommand::SendPeerMessageOnConnection {
                            device_id: device_id.clone(),
                            connection_id: connection_id_for_task.clone(),
                            message: proto::PeerMessage {
                                body: Some(Body::FileChunk(proto::FileChunk {
                                    file_id: file_id.clone(),
                                    seq,
                                    data,
                                    eof,
                                    total_size,
                                    offset,
                                    sha256: digest,
                                })),
                            },
                            enqueued_at: Instant::now(),
                            reply_to: reply_tx,
                        }).map_err(|_| "Transport is unavailable".to_string())?;

                    let queue_result: Result<(), String> = tokio::select! {
                        _ = cancel.cancelled() => Err("CANCELLED".to_string()),
                        _ = interrupt.cancelled() => Err("INTERRUPTED".to_string()),
                        r = tokio::time::timeout(Duration::from_secs(15), reply_rx) => {
                            if matches!(r, Ok(Ok(true))) {
                                Ok(())
                            } else {
                                Err("Transport rejected or timed out queuing file data".to_string())
                            }
                        }
                    };
                    let queue_elapsed = t.elapsed();
                    profile.queue.record(queue_elapsed);
                    queue_result?;
                    tracing::trace!(
                        %file_id, seq, offset, chunk_bytes = n,
                        queue_us = queue_elapsed.as_micros() as u64,
                        "file upload chunk queued"
                    );

                    let t = Instant::now();
                    let ack_result: Result<(), String> = tokio::select! {
                        _ = cancel.cancelled() => Err("CANCELLED".to_string()),
                        _ = interrupt.cancelled() => Err("INTERRUPTED".to_string()),
                        ack = tokio::time::timeout(Duration::from_secs(30), ack_rx.recv()) => {
                            match ack {
                                Ok(Some(v)) if v == end => Ok(()),
                                Ok(Some(v)) => Err(format!("Unexpected transfer acknowledgement offset {v}")),
                                _ => Err("Timed out waiting for transfer acknowledgement".to_string()),
                            }
                        }
                    };
                    let ack_elapsed = t.elapsed();
                    profile.ack_wait.record(ack_elapsed);
                    ack_result?;
                    tracing::trace!(
                        %file_id, seq, confirmed_offset = end, chunk_bytes = n,
                        ack_wait_us = ack_elapsed.as_micros() as u64,
                        "file upload chunk acknowledged"
                    );

                    offset = end;
                    confirmed_offset_task.store(offset, Ordering::Release);
                    profile.cycle.record(cycle_start.elapsed());
                    profile.bytes += n as u64;

                    let state = if eof { "complete" } else { "sending" };

                    let msg = if eof {
                        format!("Sending finished: {file_name}")
                    } else {
                        format!(
                            "Sending {file_name} · {}%",
                            Self::percentage(offset, total_size)
                        )
                    };

                    let t = Instant::now();
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        offset,
                        total_size,
                        state,
                        msg,
                    )
                    .await;
                    profile.notify.record(t.elapsed());

                    if eof {
                        break;
                    }

                    seq = seq.saturating_add(1);
                }

                Ok(())
            }
            .await;
            profile.log_summary(
                &file_id,
                match &result {
                    Ok(()) => "complete",
                    Err(e) => e.as_str(),
                },
            );

            if let Err(error) = result
                && !superseded_task.load(Ordering::Acquire)
            {
                if error == "CANCELLED" {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed_offset.load(Ordering::Acquire),
                        total_size,
                        "cancelled",
                        format!("Sending {file_name} was cancelled"),
                    )
                    .await;
                } else if error == "INTERRUPTED" && paused_task.load(Ordering::Acquire) {
                    let confirmed = confirmed_offset.load(Ordering::Acquire);
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed,
                        total_size,
                        "paused",
                        format!("Paused sending {file_name}"),
                    )
                    .await;
                } else if error == "INTERRUPTED"
                    || error.starts_with("Transport")
                    || error.starts_with("Timed out")
                {
                    let confirmed = confirmed_offset.load(Ordering::Acquire);

                    Self::emit_transfer(
                    &events_tx,
                    &notification,
                    &peer_id,
                    &peer_name,
                    can_pause,
                    &file_id,
                    &file_name,
                    "send",
                    confirmed,
                    total_size,
                    "interrupted",
                    format!(
                        "Sending {file_name} was interrupted; it can resume from {confirmed} bytes"
                    ),
                )
                .await;
                } else {
                    Self::emit_transfer(
                        &events_tx,
                        &notification,
                        &peer_id,
                        &peer_name,
                        can_pause,
                        &file_id,
                        &file_name,
                        "send",
                        confirmed_offset.load(Ordering::Acquire),
                        total_size,
                        "failed",
                        error,
                    )
                    .await;
                }
            }

            let _ = done_tx.send((key.0, key.1, session_id));
        });
    }

    pub(super) fn on_file_chunk_ack(&mut self, device_id: String, ack: proto::FileChunkAck) {
        tracing::trace!(%device_id, file_id = %ack.file_id, confirmed_offset = ack.confirmed_offset, "file upload acknowledgement received");
        if let Some(session) = self.upload_sessions.get(&(device_id, ack.file_id)) {
            let _ = session.ack_tx.send(ack.confirmed_offset);
        }
    }

    pub(super) fn on_file_transfer_cancel(
        &mut self,
        device_id: String,
        cancel: proto::FileTransferCancel,
    ) {
        // The peer stopped receiving from us. A pause keeps the receiver's
        // partial data, so end our stream as an interruption, not a cancel.
        if let Some(session) = self
            .upload_sessions
            .get(&(device_id.clone(), cancel.file_id.clone()))
        {
            if cancel.pause {
                session.interrupt.cancel();
            } else {
                session.cancel.cancel();
            }
        }
        // The peer stopped sending to us.
        let ours = self
            .pending_downloads
            .get(&cancel.file_id)
            .is_some_and(|p| p.device_id == device_id);
        if ours && let Some(p) = self.release_pending_download(&cancel.file_id) {
            if cancel.pause {
                // Kept for an explicit resume (ours, or the sender's
                // FileTransferResume), never restarted on its own.
                let _ = fs::write(
                    self.incoming_paused_path(&cancel.file_id),
                    Self::now_ms().to_string(),
                );
                let message = format!("{} was paused by the sender", p.file_name);
                self.park_download(&p, &cancel.file_id, true, "paused", message);
            } else {
                let message = format!("The sender cancelled {}", p.file_name);
                self.park_download(&p, &cancel.file_id, false, "cancelled", message);
            }
            return;
        }
        if !cancel.pause {
            // May be a partial we kept (a paused download) that the sender
            // has now given up on.
            self.discard_kept_partial(&device_id, &cancel.file_id);
        }
    }

    pub(super) fn cancel_local_transfer(&mut self, file_id: &str) {
        let keys: Vec<(String, String)> = self
            .upload_sessions
            .keys()
            .filter(|(_, id)| id == file_id)
            .cloned()
            .collect();
        for k in keys {
            if let Some(s) = self.upload_sessions.remove(&k) {
                s.cancel.cancel();
            }
        }
        self.cancel_local_download(file_id);
    }
}
