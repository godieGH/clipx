//! The IPC side of the notification center: turning hub events into IPC events
//! and answering the center's requests. Kept apart from the socket loop so it
//! can be tested without a connection.

use crate::device::manager::{DeviceCommands, TransferAction};
use crate::message::proto::clipx;
use crate::notification::{
    IncomingClipboardKind,
    ipc_engine::{ClosedReason, Decision, HubEvent, PromptHub, PromptInfo, PromptKind},
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

pub fn prompt_to_ipc(info: PromptInfo) -> clipx::IpcEvent {
    use clipx::notification_prompt_event::{ContentKind, Kind};
    let kind = match info.kind {
        PromptKind::PairRequest => Kind::PairRequest,
        PromptKind::ConfirmCode => Kind::ConfirmCode,
        PromptKind::IncomingClipboard => Kind::IncomingClipboard,
    };
    let content_kind = match info.content_kind {
        IncomingClipboardKind::Text => ContentKind::Text,
        IncomingClipboardKind::RichText => ContentKind::RichText,
        IncomingClipboardKind::Image => ContentKind::Image,
        IncomingClipboardKind::File => ContentKind::File,
    };
    clipx::IpcEvent {
        event: Some(clipx::ipc_event::Event::NotificationPrompt(
            clipx::NotificationPromptEvent {
                prompt_id: info.prompt_id,
                kind: kind as i32,
                peer_name: info.peer_name,
                code: info.code,
                content: info.content,
                content_kind: content_kind as i32,
                timeout_ms: info.timeout_ms,
                shown_at_ms: info.shown_at_ms,
            },
        )),
    }
}

pub fn hub_event_to_ipc(event: HubEvent) -> clipx::IpcEvent {
    match event {
        HubEvent::Prompt(info) => prompt_to_ipc(info),
        HubEvent::Closed { prompt_id, reason } => {
            use clipx::notification_closed_event::Reason;
            let reason = match reason {
                ClosedReason::Answered => Reason::Answered,
                ClosedReason::TimedOut => Reason::TimedOut,
            };
            clipx::IpcEvent {
                event: Some(clipx::ipc_event::Event::NotificationClosed(
                    clipx::NotificationClosedEvent {
                        prompt_id,
                        reason: reason as i32,
                    },
                )),
            }
        }
        HubEvent::Info {
            id,
            title,
            body,
            at_ms,
        } => clipx::IpcEvent {
            event: Some(clipx::ipc_event::Event::NotificationInfo(
                clipx::NotificationInfoEvent {
                    id,
                    title,
                    body,
                    at_ms,
                },
            )),
        },
    }
}

pub fn resolve_prompt_response(
    hub: &PromptHub,
    req: clipx::ResolvePromptRequest,
) -> clipx::ResolvePromptResponse {
    use clipx::resolve_prompt_request::Decision as Wire;
    let decision = match Wire::try_from(req.decision) {
        Ok(Wire::Allow) => Decision::Allow,
        Ok(Wire::Deny) => Decision::Deny,
        Ok(Wire::Copy) => Decision::Copy,
        Ok(Wire::Ignore) => Decision::Ignore,
        Err(_) => {
            return clipx::ResolvePromptResponse {
                resolved: false,
                message: "Unknown answer".into(),
            };
        }
    };
    match hub.resolve(&req.prompt_id, decision) {
        Ok(()) => clipx::ResolvePromptResponse {
            resolved: true,
            message: String::new(),
        },
        Err(message) => clipx::ResolvePromptResponse {
            resolved: false,
            message,
        },
    }
}

pub async fn control_transfer_response(
    device_tx: &UnboundedSender<DeviceCommands>,
    req: clipx::ControlTransferRequest,
) -> clipx::ControlTransferResponse {
    use clipx::control_transfer_request::Action as Wire;
    let action = match Wire::try_from(req.action) {
        Ok(Wire::Pause) => TransferAction::Pause,
        Ok(Wire::Resume) => TransferAction::Resume,
        Ok(Wire::Cancel) => TransferAction::Cancel,
        Err(_) => {
            return clipx::ControlTransferResponse {
                ok: false,
                message: "Unknown action".into(),
            };
        }
    };
    let (reply_to, reply) = oneshot::channel();
    let sent = device_tx.send(DeviceCommands::ControlTransfer {
        device_id: (!req.device_id.is_empty()).then_some(req.device_id),
        file_id: req.file_id,
        action,
        reply_to,
    });
    let outcome = match (sent,) {
        (Ok(()),) => reply.await.map_err(|_| "The core is stopping".to_string()),
        (Err(_),) => Err("The core is stopping".to_string()),
    };
    match outcome {
        Ok(Ok(message)) => clipx::ControlTransferResponse { ok: true, message },
        Ok(Err(message)) | Err(message) => clipx::ControlTransferResponse { ok: false, message },
    }
}

pub async fn list_transfers_response(
    device_tx: &UnboundedSender<DeviceCommands>,
) -> clipx::ListTransfersResponse {
    let (reply_to, reply) = oneshot::channel();
    let rows = if device_tx
        .send(DeviceCommands::ListTransfers { reply_to })
        .is_ok()
    {
        reply.await.unwrap_or_default()
    } else {
        Vec::new()
    };
    clipx::ListTransfersResponse {
        transfers: rows
            .into_iter()
            .map(|t| clipx::TransferInfo {
                device_id: t.device_id,
                device_name: t.device_name,
                entry_id: t.entry_id,
                file_id: t.file_id,
                file_name: t.file_name,
                direction: t.direction,
                done: t.done,
                total: t.total,
                state: t.state,
                can_pause: t.can_pause,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{
        PairDecision, Prompt, ipc_engine::IpcNotificationEngine,
        platform::NotificationEngine as Engine,
    };
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct Quiet;
    impl Engine for Quiet {
        fn ask_pair<'a>(
            &'a self,
            _: Prompt,
            _: Duration,
        ) -> crate::notification::platform::NotificationFuture<'a, PairDecision> {
            Box::pin(async { PairDecision::NoResponse })
        }
        fn ask_clipboard<'a>(
            &'a self,
            _: Prompt,
            _: Duration,
        ) -> crate::notification::platform::NotificationFuture<
            'a,
            crate::notification::IncomingClipboardDecision,
        > {
            Box::pin(async { crate::notification::IncomingClipboardDecision::Ignore })
        }
        fn notify_info<'a>(
            &'a self,
            _: &'a str,
            _: String,
        ) -> crate::notification::platform::NotificationFuture<'a, ()> {
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn a_prompt_round_trips_through_the_ipc_types() {
        let engine = IpcNotificationEngine::new(Quiet);
        let hub = engine.hub();
        let (_guard, mut rx, _) = hub.attach();
        let asked = tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .ask_pair(
                        Prompt::ConfirmCode {
                            peer_name: "Pixel".into(),
                            code: "123 456".into(),
                        },
                        Duration::from_secs(5),
                    )
                    .await
            }
        });

        let event = loop {
            if let HubEvent::Prompt(info) = rx.recv().await.unwrap() {
                break hub_event_to_ipc(HubEvent::Prompt(info));
            }
        };
        let Some(clipx::ipc_event::Event::NotificationPrompt(p)) = event.event else {
            panic!("expected a prompt event");
        };
        assert_eq!(
            p.kind,
            clipx::notification_prompt_event::Kind::ConfirmCode as i32
        );
        assert_eq!(
            (p.peer_name.as_str(), p.code.as_str()),
            ("Pixel", "123 456")
        );
        assert_eq!(p.timeout_ms, 5000);

        let response = resolve_prompt_response(
            &hub,
            clipx::ResolvePromptRequest {
                prompt_id: p.prompt_id.clone(),
                decision: clipx::resolve_prompt_request::Decision::Allow as i32,
            },
        );
        assert!(response.resolved, "{}", response.message);
        assert_eq!(asked.await.unwrap(), PairDecision::Allow);

        // A second answer, a wrong kind of answer and a nonsense answer are all refused.
        let again = resolve_prompt_response(
            &hub,
            clipx::ResolvePromptRequest {
                prompt_id: p.prompt_id,
                decision: clipx::resolve_prompt_request::Decision::Deny as i32,
            },
        );
        assert!(!again.resolved && !again.message.is_empty());
        let nonsense = resolve_prompt_response(
            &hub,
            clipx::ResolvePromptRequest {
                prompt_id: "x".into(),
                decision: 99,
            },
        );
        assert!(!nonsense.resolved);
    }

    #[test]
    fn closed_and_info_events_convert() {
        let closed = hub_event_to_ipc(HubEvent::Closed {
            prompt_id: "p1".into(),
            reason: ClosedReason::TimedOut,
        });
        match closed.event {
            Some(clipx::ipc_event::Event::NotificationClosed(c)) => {
                assert_eq!(c.prompt_id, "p1");
                assert_eq!(
                    c.reason,
                    clipx::notification_closed_event::Reason::TimedOut as i32
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let info = hub_event_to_ipc(HubEvent::Info {
            id: "i1".into(),
            title: "Image saved".into(),
            body: "/tmp/a.png".into(),
            at_ms: 7,
        });
        match info.event {
            Some(clipx::ipc_event::Event::NotificationInfo(i)) => {
                assert_eq!((i.title.as_str(), i.at_ms), ("Image saved", 7));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn control_transfer_reaches_the_device_manager_and_returns_its_answer() {
        let (device_tx, mut device_rx) = mpsc::unbounded_channel();
        let manager = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(cmd) = device_rx.recv().await {
                if let DeviceCommands::ControlTransfer {
                    device_id,
                    file_id,
                    action,
                    reply_to,
                } = cmd
                {
                    let refuse = action == TransferAction::Pause;
                    seen.push((device_id, file_id, action));
                    let _ = reply_to.send(if refuse {
                        Err("older ClipX".to_string())
                    } else {
                        Ok("transfer cancelled".to_string())
                    });
                    if seen.len() == 2 {
                        break;
                    }
                }
            }
            seen
        });

        let cancelled = control_transfer_response(
            &device_tx,
            clipx::ControlTransferRequest {
                file_id: "f1".into(),
                device_id: "dev-a".into(),
                action: clipx::control_transfer_request::Action::Cancel as i32,
            },
        )
        .await;
        assert!(cancelled.ok);
        assert_eq!(cancelled.message, "transfer cancelled");

        let refused = control_transfer_response(
            &device_tx,
            clipx::ControlTransferRequest {
                file_id: "f1".into(),
                device_id: String::new(),
                action: clipx::control_transfer_request::Action::Pause as i32,
            },
        )
        .await;
        assert!(!refused.ok);
        assert_eq!(refused.message, "older ClipX");

        let seen = manager.await.unwrap();
        assert_eq!(
            seen[0],
            (
                Some("dev-a".to_string()),
                "f1".to_string(),
                TransferAction::Cancel
            )
        );
        assert_eq!(seen[1].0, None, "an empty device id means any device");
    }

    #[tokio::test]
    async fn control_transfer_and_list_survive_a_stopped_core() {
        let (device_tx, device_rx) = mpsc::unbounded_channel();
        drop(device_rx);
        let response = control_transfer_response(
            &device_tx,
            clipx::ControlTransferRequest {
                file_id: "f".into(),
                device_id: String::new(),
                action: clipx::control_transfer_request::Action::Cancel as i32,
            },
        )
        .await;
        assert!(!response.ok);
        assert!(
            list_transfers_response(&device_tx)
                .await
                .transfers
                .is_empty()
        );
    }
}
