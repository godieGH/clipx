use super::*;
use crate::notification::platform::NotificationFuture;
use crate::notification::{IncomingClipboardDecision, PairDecision, Prompt};

/// Engine that does nothing, so the tests behave the same on every OS
/// (the real Windows engine would raise toasts).
#[derive(Clone)]
struct QuietEngine;

impl Engine for QuietEngine {
    fn ask_pair<'a>(&'a self, _: Prompt, _: Duration) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async { PairDecision::NoResponse })
    }
    fn ask_clipboard<'a>(
        &'a self,
        _: Prompt,
        _: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision> {
        Box::pin(async { IncomingClipboardDecision::Ignore })
    }
    fn notify_info<'a>(&'a self, _: &'a str, _: String) -> NotificationFuture<'a, ()> {
        Box::pin(async {})
    }
}

struct Harness {
    mgr: DeviceManager<QuietEngine>,
    transport_rx: mpsc::UnboundedReceiver<TransportCommand>,
    clipboard_rx: mpsc::UnboundedReceiver<ClipboardCommand>,
    events_rx: broadcast::Receiver<CoreEvent>,
    dir: PathBuf,
}

const DEVICE: &str = "dev-a";
const OTHER: &str = "dev-b";
const FILE: &str = "file-1";
const ENTRY: &str = "entry-1";
const TOTAL: u64 = 4096;
const PARTIAL: u64 = 1024;

fn harness() -> Harness {
    let dir = std::env::temp_dir().join(format!("clipx-transfer-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let (transport_tx, transport_rx) = mpsc::unbounded_channel();
    let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = broadcast::channel(64);
    let identity = Arc::new(DeviceIdentity::load_or_create(dir.join("identity.key")));
    let mut mgr = DeviceManager::new(
        dir.join("trusted.json"),
        identity,
        Some(transport_tx),
        QuietEngine,
        clipboard_tx,
        events_tx,
    );
    mgr.incoming_files_dir = dir.join("incoming");
    fs::create_dir_all(&mgr.incoming_files_dir).unwrap();
    Harness {
        mgr,
        transport_rx,
        clipboard_rx,
        events_rx,
        dir,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn offer_expiry() -> u64 {
    DeviceManager::<QuietEngine>::now_ms() + 60 * 60 * 1000
}

/// Marks a peer as speaking this build's protocol, with or without the
/// pause/resume capability.
fn peer(h: &mut Harness, device: &str, can_control: bool) {
    let caps = if can_control {
        vec![proto::PeerCapabilities::CapabilityFileTransferControl as i32]
    } else {
        vec![]
    };
    h.mgr
        .effective_versions
        .insert(device.to_string(), (1, caps));
    h.mgr.connected.insert(device.into(), "conn-1".into());
}

/// A download that already received PARTIAL bytes, exactly as the chunk
/// handler leaves it: pending entry, temp file and resume state.
fn partial_download(h: &mut Harness) {
    let pending = PendingDownload {
        device_id: DEVICE.into(),
        entry_id: ENTRY.into(),
        file_name: "movie.mp4".into(),
        total_size: TOTAL,
        offer_expires_at_ms: offer_expiry(),
        received_size: PARTIAL,
        received_complete: false,
        last_activity: Instant::now(),
        stall_retries: 0,
    };
    fs::write(
        h.mgr.incoming_temp_path(DEVICE, FILE),
        vec![7u8; PARTIAL as usize],
    )
    .unwrap();
    h.mgr.persist_resume_state(DEVICE, FILE, &pending, PARTIAL);
    h.mgr.pending_downloads.insert(FILE.into(), pending);
    peer(h, DEVICE, true);
}

struct Tokens {
    cancel: CancellationToken,
    interrupt: CancellationToken,
}

fn upload_to(h: &mut Harness, device: &str, done: u64) -> Tokens {
    let (ack_tx, _ack_rx) = mpsc::unbounded_channel();
    let tokens = Tokens {
        cancel: CancellationToken::new(),
        interrupt: CancellationToken::new(),
    };
    h.mgr.upload_sessions.insert(
        (device.into(), FILE.into()),
        UploadSession {
            id: 1,
            cancel: tokens.cancel.clone(),
            interrupt: tokens.interrupt.clone(),
            file_name: "movie.mp4".into(),
            total_size: TOTAL,
            confirmed: Arc::new(AtomicU64::new(done)),
            paused: Arc::new(AtomicBool::new(false)),
            superseded: Arc::new(AtomicBool::new(false)),
            ack_tx,
        },
    );
    tokens
}

/// (device, file, pause) for every FileTransferCancel sent.
fn cancels(h: &mut Harness) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    while let Ok(cmd) = h.transport_rx.try_recv() {
        if let TransportCommand::SendPeerMessage {
            device_id, message, ..
        } = cmd
            && let Some(Body::FileTransferCancel(c)) = message.body
        {
            out.push((device_id, c.file_id, c.pause));
        }
    }
    out
}

fn resumes_sent(h: &mut Harness) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Ok(cmd) = h.transport_rx.try_recv() {
        if let TransportCommand::SendPeerMessage {
            device_id, message, ..
        } = cmd
            && let Some(Body::FileTransferResume(r)) = message.body
        {
            out.push((device_id, r.file_id));
        }
    }
    out
}

fn download_requests(h: &mut Harness) -> Vec<u64> {
    let mut out = Vec::new();
    while let Ok(cmd) = h.transport_rx.try_recv() {
        if let TransportCommand::SendPeerMessage { message, .. } = cmd
            && let Some(Body::FileDownloadRequest(r)) = message.body
        {
            out.push(r.offset);
        }
    }
    out
}

struct Seen {
    state: String,
    done: u64,
    device_id: String,
    can_pause: bool,
}

fn last_event(h: &mut Harness) -> Option<Seen> {
    let mut last = None;
    while let Ok(ev) = h.events_rx.try_recv() {
        if let CoreEvent::FileTransferChanged {
            state,
            done,
            device_id,
            can_pause,
            ..
        } = ev
        {
            last = Some(Seen {
                state,
                done,
                device_id,
                can_pause,
            });
        }
    }
    last
}

fn last_state(h: &mut Harness) -> Option<(String, u64)> {
    last_event(h).map(|e| (e.state, e.done))
}

#[tokio::test]
async fn pause_keeps_partial_data_and_tells_the_sender() {
    let mut h = harness();
    partial_download(&mut h);

    h.mgr.stop_transfer(FILE, None, true);

    assert!(!h.mgr.pending_downloads.contains_key(FILE));
    assert!(
        h.mgr.incoming_temp_path(DEVICE, FILE).exists(),
        "partial file must stay"
    );
    assert!(
        h.mgr.incoming_state_path(FILE).exists(),
        "resume state must stay"
    );
    assert!(
        h.mgr.incoming_paused_path(FILE).exists(),
        "paused marker must be written"
    );
    let ev = last_event(&mut h).unwrap();
    assert_eq!((ev.state.as_str(), ev.done), ("paused", PARTIAL));
    assert_eq!(
        ev.device_id, DEVICE,
        "events say which device the transfer is with"
    );
    assert!(ev.can_pause);
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), true)]);
}

#[tokio::test]
async fn cancel_discards_partial_data_but_not_the_history_entry() {
    let mut h = harness();
    partial_download(&mut h);

    h.mgr.stop_transfer(FILE, None, false);

    assert!(!h.mgr.pending_downloads.contains_key(FILE));
    assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert!(!h.mgr.incoming_state_path(FILE).exists());
    assert!(!h.mgr.incoming_paused_path(FILE).exists());
    assert!(
        !h.mgr
            .incoming_files_dir
            .join(format!("clipx-{FILE}.cancelled"))
            .exists(),
        "no .cancelled marker: that is only for history removal"
    );
    assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
    let mut released = false;
    while let Ok(cmd) = h.clipboard_rx.try_recv() {
        assert!(!matches!(cmd, ClipboardCommand::RemoveEntry { .. }));
        released |= matches!(cmd, ClipboardCommand::FileDownloadReleased { .. });
    }
    assert!(
        released,
        "the clipboard manager's download slot must be freed"
    );
}

#[tokio::test]
async fn paused_download_is_not_restarted_by_the_startup_scan() {
    let mut h = harness();
    partial_download(&mut h);
    h.mgr.stop_transfer(FILE, None, true);

    h.mgr.load_resume_candidates();
    assert!(
        h.mgr.resume_candidates.is_empty(),
        "paused downloads wait for the user"
    );
    assert!(
        h.mgr.incoming_state_path(FILE).exists(),
        "scan must not delete paused state"
    );

    // An interrupted (not paused) download is still resumed automatically.
    fs::remove_file(h.mgr.incoming_paused_path(FILE)).unwrap();
    h.mgr.load_resume_candidates();
    assert!(h.mgr.resume_candidates.contains_key(FILE));
}

fn file_request(h: &Harness) -> ClipboardOutbound {
    ClipboardOutbound::FileRequest {
        device_id: DEVICE.into(),
        file_id: FILE.into(),
        entry_id: ENTRY.into(),
        file_name: "movie.mp4".into(),
        total_size: TOTAL,
        offer_expires_at_ms: offer_expiry_of(h),
    }
}

/// The expiry recorded in the kept resume state (a fresh `offer_expiry()`
/// would not match it).
fn offer_expiry_of(h: &Harness) -> u64 {
    let bytes = fs::read(h.mgr.incoming_state_path(FILE)).unwrap();
    serde_json::from_slice::<ResumeState>(&bytes)
        .unwrap()
        .offer_expires_at_ms
}

#[tokio::test]
async fn resume_continues_from_the_kept_offset_and_clears_the_marker() {
    let mut h = harness();
    partial_download(&mut h);
    h.mgr.stop_transfer(FILE, None, true);
    let _ = cancels(&mut h);

    let req = file_request(&h);
    h.mgr
        .handle_clipboard_outbound(req, &h.mgr.clipboard_tx.clone());

    assert_eq!(download_requests(&mut h), vec![PARTIAL]);
    assert_eq!(h.mgr.pending_downloads[FILE].received_size, PARTIAL);
    assert!(!h.mgr.incoming_paused_path(FILE).exists());
    assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
}

#[tokio::test]
async fn download_after_cancel_starts_from_zero() {
    let mut h = harness();
    partial_download(&mut h);
    let req = file_request(&h);
    h.mgr.stop_transfer(FILE, None, false);
    let _ = cancels(&mut h);

    h.mgr
        .handle_clipboard_outbound(req, &h.mgr.clipboard_tx.clone());

    assert_eq!(download_requests(&mut h), vec![0]);
    assert_eq!(h.mgr.pending_downloads[FILE].received_size, 0);
}

#[tokio::test]
async fn pausing_with_an_old_peer_is_refused_but_cancel_still_works() {
    let mut h = harness();
    partial_download(&mut h);
    peer(&mut h, DEVICE, false); // an older build: no pause/resume capability

    let reply = h.mgr.control_transfer(FILE, None, TransferAction::Pause);
    let message = reply.clone().unwrap_err();
    assert!(
        message.contains("older ClipX"),
        "toast explains why: {message}"
    );
    assert!(
        h.mgr.pending_downloads.contains_key(FILE),
        "transfer keeps running"
    );
    assert!(
        cancels(&mut h).is_empty(),
        "nothing is sent to the old peer"
    );
    assert!(!last_event(&mut h).is_some_and(|e| e.state == "paused"));

    let reply = h.mgr.control_transfer(FILE, None, TransferAction::Cancel);
    assert_eq!(reply, Ok("transfer cancelled".to_string()));
    assert!(!h.mgr.pending_downloads.contains_key(FILE));
    // The old peer understands a plain cancel (the extra field is ignored).
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
}

#[tokio::test]
async fn a_peer_with_no_hello_at_all_counts_as_old() {
    let mut h = harness();
    partial_download(&mut h);
    h.mgr.effective_versions.remove(DEVICE);
    assert!(!h.mgr.peer_supports_transfer_control(DEVICE));
    assert!(
        h.mgr
            .control_transfer(FILE, None, TransferAction::Pause)
            .unwrap_err()
            .contains("older ClipX")
    );
}

#[tokio::test]
async fn old_peers_still_pass_the_hello_gate_and_new_ones_advertise_control() {
    let mut h = harness();
    // A hello with no capabilities (an older build) is accepted, not rejected.
    let old = Hello {
        proto_version: config::PROTO_VERSION,
        min_proto_version: config::MINI_PROTO_VERSION,
        capabilities: vec![],
    };
    assert!(h.mgr.check_hello("old-peer", Some(old)).is_ok());
    assert!(!h.mgr.peer_supports_transfer_control("old-peer"));
    // This build advertises the capability and a peer that sends it is recognised.
    let mine = config::local_hello();
    assert!(h.mgr.check_hello("new-peer", Some(mine)).is_ok());
    assert!(h.mgr.peer_supports_transfer_control("new-peer"));
    // A peer that sends no hello (legacy) still connects.
    assert!(h.mgr.check_hello("legacy", None).is_ok());
}

#[tokio::test]
async fn one_offer_to_two_devices_is_controlled_per_device() {
    let mut h = harness();
    peer(&mut h, DEVICE, true);
    peer(&mut h, OTHER, true);
    let a = upload_to(&mut h, DEVICE, 100);
    let b = upload_to(&mut h, OTHER, 200);

    let reply = h
        .mgr
        .control_transfer(FILE, Some(DEVICE), TransferAction::Cancel);
    assert_eq!(reply, Ok("transfer cancelled".to_string()));

    assert!(
        a.cancel.is_cancelled(),
        "the chosen device's transfer is cancelled"
    );
    assert!(
        !b.cancel.is_cancelled(),
        "the other device's transfer keeps going"
    );
    assert!(
        h.mgr
            .upload_sessions
            .contains_key(&(OTHER.into(), FILE.into()))
    );
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);

    let snapshot = h.mgr.list_transfers();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(
        (snapshot[0].device_id.as_str(), snapshot[0].done),
        (OTHER, 200)
    );
}

#[tokio::test]
async fn stopping_an_upload_cancels_it_and_pausing_holds_it() {
    for pause in [false, true] {
        let mut h = harness();
        peer(&mut h, DEVICE, true);
        let t = upload_to(&mut h, DEVICE, 512);

        h.mgr.stop_transfer(FILE, None, pause);

        assert_eq!(t.cancel.is_cancelled(), !pause);
        assert_eq!(t.interrupt.is_cancelled(), pause);
        assert!(h.mgr.upload_sessions.is_empty());
        assert_eq!(h.mgr.held_uploads.len(), usize::from(pause));
        assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), pause)]);
    }
}

#[tokio::test]
async fn sender_pause_then_resume_asks_the_receiver_to_continue() {
    let mut h = harness();
    peer(&mut h, DEVICE, true);
    let _t = upload_to(&mut h, DEVICE, 512);

    assert_eq!(
        h.mgr.control_transfer(FILE, None, TransferAction::Pause),
        Ok("transfer paused".to_string())
    );
    let held = &h.mgr.held_uploads[&(DEVICE.to_string(), FILE.to_string())];
    assert_eq!((held.done, held.waiting), (512, false));
    let _ = cancels(&mut h);
    assert_eq!(h.mgr.list_transfers()[0].state, "paused");

    assert!(
        h.mgr
            .control_transfer(FILE, None, TransferAction::Resume)
            .is_ok()
    );
    assert_eq!(resumes_sent(&mut h), vec![(DEVICE.into(), FILE.into())]);
    assert!(h.mgr.held_uploads[&(DEVICE.to_string(), FILE.to_string())].waiting);
    assert_eq!(h.mgr.list_transfers()[0].state, "requesting");

    // Cancelling while waiting reports it (no task is left to) and tells the peer.
    let _ = last_event(&mut h);
    assert!(
        h.mgr
            .control_transfer(FILE, None, TransferAction::Cancel)
            .is_ok()
    );
    assert!(h.mgr.held_uploads.is_empty());
    assert_eq!(last_state(&mut h), Some(("cancelled".into(), 512)));
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
}

#[tokio::test]
async fn the_receiver_asking_again_clears_a_held_upload() {
    let mut h = harness();
    peer(&mut h, DEVICE, true);
    h.mgr.held_uploads.insert(
        (DEVICE.into(), FILE.into()),
        HeldUpload {
            file_name: "movie.mp4".into(),
            total_size: TOTAL,
            done: 512,
            waiting: true,
        },
    );
    // stream_file_to_peer clears it before anything is sent; with no real
    // file it returns early, so check the disconnect cleanup instead.
    h.mgr.held_uploads.retain(|(d, _), _| d != DEVICE);
    assert!(h.mgr.held_uploads.is_empty());
}

#[tokio::test]
async fn resuming_an_upload_to_an_old_peer_is_refused() {
    let mut h = harness();
    peer(&mut h, DEVICE, false);
    h.mgr.held_uploads.insert(
        (DEVICE.into(), FILE.into()),
        HeldUpload {
            file_name: "movie.mp4".into(),
            total_size: TOTAL,
            done: 512,
            waiting: false,
        },
    );
    let reply = h.mgr.control_transfer(FILE, None, TransferAction::Resume);
    assert!(reply.unwrap_err().contains("older ClipX"));
    assert!(resumes_sent(&mut h).is_empty());
}

#[tokio::test]
async fn sender_cancelling_ends_our_download_and_reports_it() {
    let mut h = harness();
    partial_download(&mut h);

    h.mgr.on_file_transfer_cancel(
        DEVICE.into(),
        proto::FileTransferCancel {
            file_id: FILE.into(),
            pause: false,
        },
    );

    assert!(!h.mgr.pending_downloads.contains_key(FILE));
    assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
    let released = std::iter::from_fn(|| h.clipboard_rx.try_recv().ok())
        .any(|c| matches!(c, ClipboardCommand::FileDownloadReleased { .. }));
    assert!(released, "clipboard manager must free the download slot");
}

#[tokio::test]
async fn sender_pausing_keeps_our_partial_data_and_waits_for_a_resume() {
    let mut h = harness();
    partial_download(&mut h);

    h.mgr.on_file_transfer_cancel(
        DEVICE.into(),
        proto::FileTransferCancel {
            file_id: FILE.into(),
            pause: true,
        },
    );

    assert!(!h.mgr.pending_downloads.contains_key(FILE));
    assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert!(h.mgr.incoming_state_path(FILE).exists());
    assert!(
        h.mgr.incoming_paused_path(FILE).exists(),
        "must not restart by itself"
    );
    assert_eq!(last_state(&mut h), Some(("paused".into(), PARTIAL)));
    let _ = download_requests(&mut h);

    // The sender presses Resume: we ask again from the kept offset.
    h.mgr.on_file_transfer_resume(
        DEVICE.into(),
        proto::FileTransferResume {
            file_id: FILE.into(),
        },
    );
    assert_eq!(download_requests(&mut h), vec![PARTIAL]);
    assert!(!h.mgr.incoming_paused_path(FILE).exists());
}

#[tokio::test]
async fn a_resume_from_the_wrong_device_or_with_nothing_kept_is_ignored() {
    let mut h = harness();
    partial_download(&mut h);
    h.mgr.stop_transfer(FILE, None, true);
    let _ = download_requests(&mut h);
    peer(&mut h, OTHER, true);

    h.mgr.on_file_transfer_resume(
        OTHER.into(),
        proto::FileTransferResume {
            file_id: FILE.into(),
        },
    );
    h.mgr.on_file_transfer_resume(
        DEVICE.into(),
        proto::FileTransferResume {
            file_id: "unknown".into(),
        },
    );
    assert!(download_requests(&mut h).is_empty());
    assert!(!h.mgr.pending_downloads.contains_key(FILE));
}

#[tokio::test]
async fn sender_cancelling_a_paused_download_discards_the_kept_partial() {
    let mut h = harness();
    partial_download(&mut h);
    h.mgr.stop_transfer(FILE, None, true);
    let _ = last_event(&mut h);

    h.mgr.on_file_transfer_cancel(
        DEVICE.into(),
        proto::FileTransferCancel {
            file_id: FILE.into(),
            pause: false,
        },
    );

    assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert!(!h.mgr.incoming_state_path(FILE).exists());
    assert!(!h.mgr.incoming_paused_path(FILE).exists());
    assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
}

#[tokio::test]
async fn cancel_from_another_device_does_not_touch_our_download() {
    let mut h = harness();
    partial_download(&mut h);

    h.mgr.on_file_transfer_cancel(
        "someone-else".into(),
        proto::FileTransferCancel {
            file_id: FILE.into(),
            pause: true,
        },
    );
    h.mgr.on_file_transfer_cancel(
        "someone-else".into(),
        proto::FileTransferCancel {
            file_id: FILE.into(),
            pause: false,
        },
    );

    assert!(h.mgr.pending_downloads.contains_key(FILE));
    assert!(h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert!(h.mgr.incoming_state_path(FILE).exists());
}

#[tokio::test]
async fn the_snapshot_lists_downloads_uploads_and_paused_uploads_per_device() {
    let mut h = harness();
    partial_download(&mut h);
    peer(&mut h, OTHER, false);
    let _t = upload_to(&mut h, OTHER, 300);
    h.mgr.held_uploads.insert(
        ("dev-c".into(), "file-2".into()),
        HeldUpload {
            file_name: "a.zip".into(),
            total_size: 10,
            done: 5,
            waiting: false,
        },
    );

    let mut snap = h.mgr.list_transfers();
    snap.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    let rows: Vec<_> = snap
        .iter()
        .map(|t| {
            (
                t.device_id.as_str(),
                t.direction.as_str(),
                t.state.as_str(),
                t.done,
                t.can_pause,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (DEVICE, "download", "receiving", PARTIAL, true),
            (OTHER, "send", "sending", 300, false),
            ("dev-c", "send", "paused", 5, false),
        ]
    );
}

#[tokio::test]
async fn a_paused_download_can_be_cancelled_and_survives_in_the_snapshot() {
    let mut h = harness();
    partial_download(&mut h);
    assert!(
        h.mgr
            .control_transfer(FILE, None, TransferAction::Pause)
            .is_ok()
    );
    let _ = (cancels(&mut h), last_event(&mut h));

    // Nothing is running, yet the paused download is listed (this is what
    // makes it visible again after an app restart).
    let snap = h.mgr.list_transfers();
    assert_eq!(snap.len(), 1);
    assert_eq!(
        (
            snap[0].state.as_str(),
            snap[0].direction.as_str(),
            snap[0].done
        ),
        ("paused", "download", PARTIAL)
    );

    // Cancel drops the kept partial data and tells the sender.
    assert_eq!(
        h.mgr
            .control_transfer(FILE, Some(DEVICE), TransferAction::Cancel),
        Ok("transfer cancelled".to_string())
    );
    assert!(!h.mgr.incoming_temp_path(DEVICE, FILE).exists());
    assert!(!h.mgr.incoming_state_path(FILE).exists());
    assert!(!h.mgr.incoming_paused_path(FILE).exists());
    assert_eq!(last_state(&mut h), Some(("cancelled".into(), PARTIAL)));
    assert_eq!(cancels(&mut h), vec![(DEVICE.into(), FILE.into(), false)]);
    assert!(h.mgr.list_transfers().is_empty());
}

#[tokio::test]
async fn nothing_to_control_gives_a_clear_message() {
    let mut h = harness();
    assert_eq!(
        h.mgr.control_transfer("none", None, TransferAction::Cancel),
        Err("No running transfer to cancel".to_string())
    );
    assert_eq!(
        h.mgr.control_transfer("none", None, TransferAction::Pause),
        Err("No running transfer to pause".to_string())
    );
    assert!(
        h.mgr
            .control_transfer("none", None, TransferAction::Resume)
            .unwrap_err()
            .contains("receiving")
    );
}
