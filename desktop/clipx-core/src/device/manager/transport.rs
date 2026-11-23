use super::*;

impl<E: Engine + Clone + 'static> DeviceManager<E> {
    pub(super) fn send_peer(&self, device_id: &str, body: Body) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };

        let device_id = device_id.to_string();

        let (reply_tx, reply_rx) = oneshot::channel();

        if tx
            .send(TransportCommand::SendPeerMessage {
                device_id: device_id.clone(),
                message: proto::PeerMessage { body: Some(body) },
                enqueued_at: Instant::now(),
                reply_to: reply_tx,
            })
            .is_err()
        {
            tracing::warn!(%device_id, "transport command channel is closed");
            return;
        }

        tokio::spawn(async move {
            match reply_rx.await {
                Ok(true) => {
                    tracing::debug!(%device_id, "peer message queued for transport");
                }
                Ok(false) => {
                    tracing::warn!(%device_id, "transport rejected peer message");
                }
                Err(_) => {
                    tracing::warn!(%device_id, "transport dropped peer-message reply");
                }
            }
        });
    }

    pub(super) fn send_peer_on_connection(&self, device_id: &str, connection_id: &str, body: Body) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::SendPeerMessageOnConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            message: proto::PeerMessage { body: Some(body) },
            enqueued_at: Instant::now(),
            reply_to: reply_tx,
        });
    }

    pub(super) fn send_control(&self, device_id: &str, code: u32, message: impl Into<String>) {
        self.send_peer(
            device_id,
            Body::Control(proto::PreTransportControl {
                code,
                message: message.into(),
            }),
        );
    }

    pub(super) fn send_control_on_connection(
        &self,
        device_id: &str,
        connection_id: &str,
        code: u32,
        message: impl Into<String>,
    ) {
        self.send_peer_on_connection(
            device_id,
            connection_id,
            Body::Control(proto::PreTransportControl {
                code,
                message: message.into(),
            }),
        );
    }

    pub(super) fn request_transport_disconnect(&self, device_id: &str) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::Disconnect {
            device_id: device_id.to_string(),
            reply_to: reply_tx,
        });
    }

    pub(super) fn promote_transport_connection(&self, device_id: &str, connection_id: &str) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::PromoteConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            reply_to: reply_tx,
        });
    }

    pub(super) fn request_transport_disconnect_on_connection(
        &self,
        device_id: &str,
        connection_id: &str,
    ) {
        let Some(tx) = self.transport_tx.as_ref() else {
            return;
        };
        let (reply_tx, _reply_rx) = oneshot::channel();
        let _ = tx.send(TransportCommand::DisconnectOnConnection {
            device_id: device_id.to_string(),
            connection_id: connection_id.to_string(),
            reply_to: reply_tx,
        });
    }

    pub(super) fn notify_info(&self, title: &'static str, body: impl Into<String>) {
        let engine = self.notification.clone();
        let body = body.into();
        tokio::spawn(async move {
            engine.notify_info(title, body).await;
        });
    }
}
