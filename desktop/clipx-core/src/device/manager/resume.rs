use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn incoming_temp_path(&self, device_id: &str, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{device_id}-{file_id}"))
    }

    pub(super) fn incoming_state_path(&self, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{file_id}.resume"))
    }

    /// Marks a download the user paused: its partial data and resume state are
    /// kept, but it must not restart by itself on reconnect or app restart.
    pub(super) fn incoming_paused_path(&self, file_id: &str) -> PathBuf {
        self.incoming_files_dir
            .join(format!("clipx-{file_id}.paused"))
    }

    pub(super) fn persist_resume_state(
        &self,
        device_id: &str,
        file_id: &str,
        pending: &PendingDownload,
        offset: u64,
    ) {
        let state = ResumeState {
            version: 1,
            device_id: device_id.to_string(),
            file_id: file_id.to_string(),
            entry_id: pending.entry_id.clone(),
            file_name: pending.file_name.clone(),
            total_size: pending.total_size,
            confirmed_offset: offset,
            offer_expires_at_ms: pending.offer_expires_at_ms,
            updated_at_ms: Self::now_ms(),
        };
        let path = self.incoming_state_path(file_id);
        let tmp = path.with_extension("resume.tmp");
        if let Ok(bytes) = serde_json::to_vec(&state)
            && fs::write(&tmp, bytes).is_ok()
        {
            let _ = fs::rename(tmp, path);
        }
    }

    pub(super) fn load_valid_resume_state(
        &self,
        file_id: &str,
        device_id: &str,
        entry_id: &str,
        file_name: &str,
        total_size: u64,
        offer_expires_at_ms: u64,
    ) -> Option<ResumeState> {
        let state: ResumeState =
            serde_json::from_slice(&fs::read(self.incoming_state_path(file_id)).ok()?).ok()?;
        let valid = state.version == 1
            && state.file_id == file_id
            && state.device_id == device_id
            && state.entry_id == entry_id
            && state.file_name == file_name
            && state.total_size == total_size
            && state.offer_expires_at_ms == offer_expires_at_ms
            && state.confirmed_offset <= total_size
            && state.offer_expires_at_ms >= Self::now_ms()
            && state
                .updated_at_ms
                .saturating_add(RESUME_STATE_TTL.as_millis() as u64)
                >= Self::now_ms()
            && fs::metadata(self.incoming_temp_path(device_id, file_id))
                .is_ok_and(|m| m.len() == state.confirmed_offset);
        if valid { Some(state) } else { None }
    }

    pub(super) fn load_resume_candidates(&mut self) {
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("resume") {
                continue;
            }
            let file_id = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .strip_prefix("clipx-")
                .unwrap_or_default()
                .to_string();
            if file_id.is_empty() {
                let _ = fs::remove_file(path);
                continue;
            }
            if self
                .incoming_files_dir
                .join(format!("clipx-{file_id}.cancelled"))
                .exists()
            {
                let _ = fs::remove_file(&path);
                continue;
            }
            // Paused by the user: keep the files, but wait for an explicit
            // resume instead of restarting on the next connection.
            if self.incoming_paused_path(&file_id).exists() {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Ok(state) = serde_json::from_slice::<ResumeState>(&bytes) else {
                let _ = fs::remove_file(path);
                continue;
            };
            let valid = state.version == 1
                && state.file_id == file_id
                && state.confirmed_offset <= state.total_size
                && state.offer_expires_at_ms >= Self::now_ms()
                && state
                    .updated_at_ms
                    .saturating_add(RESUME_STATE_TTL.as_millis() as u64)
                    >= Self::now_ms()
                && fs::metadata(self.incoming_temp_path(&state.device_id, &state.file_id))
                    .is_ok_and(|m| m.len() == state.confirmed_offset);
            if valid {
                self.resume_candidates.insert(state.file_id.clone(), state);
            } else {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(self.incoming_temp_path(&state.device_id, &state.file_id));
            }
        }
    }

    pub(super) fn cleanup_stale_incoming_files(&mut self) {
        let now = Self::now_ms();
        let cutoff = now.saturating_sub(RESUME_STATE_TTL.as_millis() as u64);
        let Ok(entries) = fs::read_dir(&self.incoming_files_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or_default();
            if path.extension().and_then(|x| x.to_str()) == Some("cancelled") {
                let stale = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                    .is_some_and(|age| age >= RESUME_STATE_TTL);
                if stale {
                    let _ = fs::remove_file(path);
                }
                continue;
            }
            if path.extension().and_then(|x| x.to_str()) == Some("resume") {
                if let Ok(bytes) = fs::read(&path)
                    && let Ok(state) = serde_json::from_slice::<ResumeState>(&bytes)
                {
                    if state.updated_at_ms >= cutoff
                        && state.offer_expires_at_ms >= now
                        && !self
                            .incoming_files_dir
                            .join(format!("clipx-{}.cancelled", state.file_id))
                            .exists()
                    {
                        continue;
                    }
                    let _ =
                        fs::remove_file(self.incoming_temp_path(&state.device_id, &state.file_id));
                }
                let _ = fs::remove_file(path);
                continue;
            }
            if self.incoming_files.values().any(|s| s.temp_path == path)
                || name.contains(".cancelled")
            {
                continue;
            }
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age >= RESUME_STATE_TTL);
            if stale {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub(super) fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
