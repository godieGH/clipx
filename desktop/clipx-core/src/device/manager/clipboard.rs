use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    /// Dispatches clipboard payloads to every currently connected, trusted peer.
    pub(super) fn broadcast_clipboard(
        &self,
        payload: ClipboardPayload,
        file_offer: Option<proto::FileOffer>,
    ) {
        if self.connected.is_empty() {
            return;
        }
        let ids: Vec<String> = self.connected.keys().cloned().collect();
        for device_id in ids {
            let content = match (&payload, file_offer.as_ref()) {
                (ClipboardPayload::Text(text), _) => {
                    Some(clipboard_message::Content::Text(text.clone()))
                }
                (ClipboardPayload::RichText { text, html }, _) => {
                    Some(clipboard_message::Content::RichText(proto::RichText {
                        text: text.clone(),
                        html: html.clone(),
                    }))
                }
                (
                    ClipboardPayload::Image {
                        width,
                        height,
                        rgba,
                    },
                    _,
                ) => Some(clipboard_message::Content::Image(proto::ImageContent {
                    width: *width,
                    height: *height,
                    rgba: rgba.clone(),
                })),
                (ClipboardPayload::Files(_), Some(file))
                | (ClipboardPayload::FileBytes { .. }, Some(file))
                | (ClipboardPayload::FilePath { .. }, Some(file)) => {
                    Some(clipboard_message::Content::FileOffer(file.clone()))
                }
                (ClipboardPayload::Files(_), None)
                | (ClipboardPayload::FileBytes { .. }, None)
                | (ClipboardPayload::FilePath { .. }, None) => None,
            };
            if let Some(content) = content {
                self.send_peer(
                    &device_id,
                    Body::Clipboard(proto::ClipboardMessage {
                        content: Some(content),
                    }),
                );
            }
        }
    }

    pub(super) fn handle_clipboard_outbound(
        &mut self,
        outbound: ClipboardOutbound,
        _clipboard_tx: &mpsc::UnboundedSender<ClipboardCommand>,
    ) {
        match outbound {
            ClipboardOutbound::Payload(payload) => self.broadcast_clipboard(payload, None),
            ClipboardOutbound::FileOffer {
                file_id,
                name,
                mime_type,
                size,
                expires_at_ms,
            } => {
                self.broadcast_clipboard(
                    ClipboardPayload::FileBytes {
                        name: name.clone(),
                        mime_type: mime_type.clone(),
                        data: Vec::new(),
                    },
                    Some(proto::FileOffer {
                        file_id,
                        name,
                        mime_type,
                        size,
                        expires_at_ms,
                    }),
                );
            }
            ClipboardOutbound::FileRequest {
                device_id,
                file_id,
                entry_id,
                file_name,
                total_size,
                offer_expires_at_ms,
            } => {
                if self.pending_downloads.contains_key(&file_id) {
                    tracing::debug!(%file_id, "ignoring duplicate file download request");
                    return;
                }
                if !self.connected.contains_key(&device_id) {
                    let _ = self
                        .clipboard_tx
                        .send(ClipboardCommand::FileDownloadReleased {
                            file_id: file_id.clone(),
                        });
                    self.notify_transfer(
                        &device_id,
                        &entry_id,
                        &file_id,
                        &file_name,
                        "download",
                        0,
                        total_size,
                        "failed",
                        "Device is not connected",
                    );
                    return;
                }
                // Continues from a kept partial download (paused or
                // interrupted) when one is still valid, else starts at 0.
                self.start_or_resume_download(
                    device_id,
                    file_id,
                    entry_id,
                    file_name,
                    total_size,
                    offer_expires_at_ms,
                );
            }
            ClipboardOutbound::FileDownloadFinished { file_id } => {
                self.pending_downloads.remove(&file_id);
            }
            ClipboardOutbound::FileTransferCancel { device_id, file_id } => {
                self.cancel_local_transfer(&file_id);
                self.send_peer(
                    &device_id,
                    Body::FileTransferCancel(proto::FileTransferCancel {
                        file_id,
                        pause: false,
                    }),
                );
            }
            ClipboardOutbound::FileStream {
                device_id,
                file_id,
                path,
                total_size,
                start_offset,
            } => self.stream_file_to_peer(device_id, file_id, path, total_size, start_offset),
        }
    }

    /// Hands a trusted peer's non-file clipboard content to ClipboardManager.
    pub(super) fn on_clipboard_message(&mut self, device_id: String, msg: proto::ClipboardMessage) {
        if !self.trusted.is_trusted(&device_id) {
            tracing::warn!("dropping clipboard content from untrusted device {device_id}");
            return;
        }
        let Some(content) = msg.content else {
            return;
        };
        let payload = match content {
            clipboard_message::Content::Text(text) => {
                if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
                    self.notify_info(
                        "Clipboard too large",
                        "Received text is larger than 4 MiB. Send it as a file instead.",
                    );
                    return;
                }
                crate::clipboard::manager::IncomingPayload::Text(text)
            }
            clipboard_message::Content::RichText(value) => {
                if value.text.len() > MAX_CLIPBOARD_TEXT_BYTES
                    || value.text.len().saturating_add(value.html.len()) > MAX_RICH_TEXT_BYTES
                {
                    self.notify_info("Clipboard too large", "Received rich text exceeds the clipboard limit. Send it as a file instead.");
                    return;
                }
                crate::clipboard::manager::IncomingPayload::RichText {
                    text: value.text,
                    html: value.html,
                }
            }
            clipboard_message::Content::Image(value) => {
                let expected = (value.width as usize)
                    .checked_mul(value.height as usize)
                    .and_then(|v| v.checked_mul(4));
                if expected != Some(value.rgba.len())
                    || value.width == 0
                    || value.height == 0
                    || value.rgba.len() > MAX_IMAGE_BYTES
                {
                    tracing::warn!("rejecting invalid clipboard image from {device_id}");
                    return;
                }
                crate::clipboard::manager::IncomingPayload::Image {
                    width: value.width,
                    height: value.height,
                    rgba: value.rgba,
                }
            }
            clipboard_message::Content::FileOffer(mut file) => {
                // The sender's own name can arrive with no extension at all
                // (this is common for the "clipboard image sent as a file"
                // path, since neither platform's picker guarantees one).
                // Fix it up once, here, so every downstream consumer — the
                // saved filename on disk, the notification title, the
                // history entry — sees a name that actually looks like what
                // it is instead of showing up as an unrecognized file.
                file.name =
                    crate::clipboard::manager::ensure_file_extension(&file.name, &file.mime_type);
                crate::clipboard::manager::IncomingPayload::FileOffer { file }
            }
        };
        let device_name = self
            .trusted
            .get(&device_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| device_id.clone());
        let _ = self.clipboard_tx.send(ClipboardCommand::IncomingRemote {
            payload,
            source_device_name: device_name,
            source_device_id: device_id,
        });
    }
}
