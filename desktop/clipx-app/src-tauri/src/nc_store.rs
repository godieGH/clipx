//! The notification center's list, kept in memory for the life of the app.
//!
//! Nothing here touches Tauri or the network: it is plain data and rules, so
//! it can be tested on its own. The glue in `notification_center.rs` feeds it
//! events from the core and the user's actions.
//!
//! "Unread" means something still needs the user or is still running: a prompt
//! waiting for an answer, a transfer in progress, a result nobody has looked
//! at. "Read" means it is dealt with: answered, expired, cancelled, finished
//! and marked read. Nothing is saved; a restart starts empty.

use serde::Serialize;

/// Most notifications kept. The oldest read ones go first; prompts waiting for
/// an answer and running transfers are never dropped to make room.
pub const MAX_NOTIFICATIONS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    PairRequest,
    ConfirmCode,
    IncomingClipboard,
    Transfer,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptStatus {
    Waiting,
    Answered,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptView {
    pub prompt_id: String,
    pub peer_name: String,
    /// Pairing code to compare, for `ConfirmCode`.
    pub code: String,
    /// Preview of what arrived, for `IncomingClipboard`.
    pub content: String,
    /// "text", "richText", "image" or "file".
    pub content_kind: String,
    pub timeout_ms: u32,
    pub expires_at_ms: u64,
    pub status: PromptStatus,
    /// How it ended: "allowed", "denied", "copied", "ignored" or "handledElsewhere".
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoView {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    pub device_id: String,
    pub device_name: String,
    pub file_id: String,
    pub file_name: String,
    /// "download" or "send".
    pub direction: String,
    pub done: u64,
    pub total: u64,
    pub state: String,
    pub message: String,
    /// False when the other device is an older build that cannot pause or resume.
    pub can_pause: bool,
}

impl TransferView {
    /// Still going, or paused by the user, so it is not for marking read or
    /// dropping. An interrupted transfer is waiting on the other device and can
    /// be dismissed; if it starts again it becomes unread again.
    pub fn is_active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "requesting" | "receiving" | "sending" | "saving" | "paused"
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub kind: Kind,
    pub unread: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub read_at_ms: Option<u64>,
    pub prompt: Option<PromptView>,
    pub info: Option<InfoView>,
    pub transfer: Option<TransferView>,
}

impl Notification {
    /// A prompt the core is still waiting on.
    pub fn is_waiting_prompt(&self) -> bool {
        self.prompt
            .as_ref()
            .is_some_and(|p| p.status == PromptStatus::Waiting)
    }

    fn is_running_transfer(&self) -> bool {
        self.transfer.as_ref().is_some_and(TransferView::is_active)
    }

    /// Cannot be marked read or dropped: the user (or the transfer) is not done with it.
    fn is_pinned(&self) -> bool {
        self.is_waiting_prompt() || self.is_running_transfer()
    }

    fn mark_read(&mut self, now: u64) {
        if self.unread {
            self.unread = false;
            self.read_at_ms = Some(now);
        }
    }

    fn mark_unread(&mut self) {
        self.unread = true;
        self.read_at_ms = None;
    }
}

/// A prompt as the core sent it.
pub struct PromptIn {
    pub prompt_id: String,
    pub kind: Kind,
    pub peer_name: String,
    pub code: String,
    pub content: String,
    pub content_kind: String,
    pub timeout_ms: u32,
}

/// One transfer event from the core.
pub struct TransferIn {
    pub device_id: String,
    pub device_name: String,
    pub file_id: String,
    pub file_name: String,
    pub direction: String,
    pub done: u64,
    pub total: u64,
    pub state: String,
    pub message: String,
    pub can_pause: bool,
}

pub fn transfer_id(device_id: &str, file_id: &str) -> String {
    format!("transfer:{device_id}|{file_id}")
}

#[derive(Default)]
pub struct Store {
    items: Vec<Notification>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unread first, oldest first (the oldest prompt is the one that expires
    /// first); then read, most recently read first.
    pub fn snapshot(&self) -> Vec<Notification> {
        let mut unread: Vec<Notification> =
            self.items.iter().filter(|n| n.unread).cloned().collect();
        unread.sort_by_key(|n| n.created_at_ms);
        let mut read: Vec<Notification> =
            self.items.iter().filter(|n| !n.unread).cloned().collect();
        read.sort_by_key(|n| std::cmp::Reverse(n.read_at_ms.unwrap_or(n.updated_at_ms)));
        unread.extend(read);
        unread
    }

    pub fn get(&self, id: &str) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    /// How many notifications need the user: what a badge would count.
    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|n| n.unread).count()
    }

    /// How many prompts are waiting for an answer.
    pub fn waiting_prompt_count(&self) -> usize {
        self.items.iter().filter(|n| n.is_waiting_prompt()).count()
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Notification> {
        self.items.iter_mut().find(|n| n.id == id)
    }

    /// A prompt arrived. A prompt id already present (the same prompt sent
    /// again after a reconnect) is left as it is.
    pub fn on_prompt(&mut self, p: PromptIn, now: u64) {
        if self.get(&p.prompt_id).is_some() {
            return;
        }
        self.items.push(Notification {
            id: p.prompt_id.clone(),
            kind: p.kind,
            unread: true,
            created_at_ms: now,
            updated_at_ms: now,
            read_at_ms: None,
            prompt: Some(PromptView {
                prompt_id: p.prompt_id,
                peer_name: p.peer_name,
                code: p.code,
                content: p.content,
                content_kind: p.content_kind,
                timeout_ms: p.timeout_ms,
                expires_at_ms: now + u64::from(p.timeout_ms),
                status: PromptStatus::Waiting,
                outcome: None,
            }),
            info: None,
            transfer: None,
        });
        self.trim();
    }

    /// The core says a prompt is over. `timed_out` means nobody answered;
    /// otherwise it was answered somewhere else (another UI, or this one a
    /// moment ago, in which case it is already closed and nothing changes).
    pub fn on_prompt_closed(&mut self, prompt_id: &str, timed_out: bool, now: u64) {
        let Some(n) = self.find_mut(prompt_id) else {
            return;
        };
        let Some(p) = n.prompt.as_mut() else { return };
        if p.status != PromptStatus::Waiting {
            return;
        }
        if timed_out {
            p.status = PromptStatus::Expired;
        } else {
            p.status = PromptStatus::Answered;
            p.outcome = Some("handledElsewhere".into());
        }
        n.updated_at_ms = now;
        n.mark_read(now);
    }

    /// The user answered a prompt here. Returns false if it was not waiting.
    pub fn on_prompt_answered(&mut self, prompt_id: &str, outcome: &str, now: u64) -> bool {
        let Some(n) = self.find_mut(prompt_id) else {
            return false;
        };
        let Some(p) = n.prompt.as_mut() else {
            return false;
        };
        if p.status != PromptStatus::Waiting {
            return false;
        }
        p.status = PromptStatus::Answered;
        p.outcome = Some(outcome.to_string());
        n.updated_at_ms = now;
        n.mark_read(now);
        true
    }

    pub fn on_info(&mut self, id: String, title: String, body: String, now: u64) {
        if self.get(&id).is_some() {
            return;
        }
        self.items.push(Notification {
            id,
            kind: Kind::Info,
            unread: true,
            created_at_ms: now,
            updated_at_ms: now,
            read_at_ms: None,
            prompt: None,
            info: Some(InfoView { title, body }),
            transfer: None,
        });
        self.trim();
    }

    /// A transfer changed. One notification per transfer (device and file), so
    /// an offer going to two devices is two. Running and finished-but-unseen
    /// transfers stay unread; a cancelled one is read, since the user (or the
    /// other side) has already ended it.
    pub fn on_transfer(&mut self, t: TransferIn, now: u64) {
        let id = transfer_id(&t.device_id, &t.file_id);
        let view = TransferView {
            device_id: t.device_id,
            device_name: t.device_name,
            file_id: t.file_id,
            file_name: t.file_name,
            direction: t.direction,
            done: t.done,
            total: t.total,
            state: t.state,
            message: t.message,
            can_pause: t.can_pause,
        };
        let cancelled = view.state == "cancelled";
        match self.find_mut(&id) {
            Some(n) => {
                n.transfer = Some(view);
                n.updated_at_ms = now;
                if cancelled {
                    n.mark_read(now);
                } else if !n.unread {
                    // Started again (a new attempt) or finished after being read.
                    n.mark_unread();
                }
            }
            None => {
                let mut n = Notification {
                    id,
                    kind: Kind::Transfer,
                    unread: true,
                    created_at_ms: now,
                    updated_at_ms: now,
                    read_at_ms: None,
                    prompt: None,
                    info: None,
                    transfer: Some(view),
                };
                if cancelled {
                    n.mark_read(now);
                }
                self.items.push(n);
                self.trim();
            }
        }
    }

    /// Adds transfers the core reports that this list has not seen (they began
    /// before the app attached). Ones already here keep their live state.
    pub fn seed_transfers(&mut self, transfers: Vec<TransferIn>, now: u64) {
        for t in transfers {
            if self.get(&transfer_id(&t.device_id, &t.file_id)).is_none() {
                self.on_transfer(t, now);
            }
        }
    }

    /// Moves a notification to read. Refused for a prompt still waiting and for
    /// a running transfer: those are not dealt with yet.
    pub fn mark_read(&mut self, id: &str, now: u64) -> bool {
        let Some(n) = self.find_mut(id) else {
            return false;
        };
        if n.is_pinned() {
            return false;
        }
        n.mark_read(now);
        true
    }

    /// Marks everything that can be marked read; returns how many changed.
    pub fn mark_all_read(&mut self, now: u64) -> usize {
        let mut changed = 0;
        for n in self.items.iter_mut().filter(|n| n.unread && !n.is_pinned()) {
            n.mark_read(now);
            changed += 1;
        }
        changed
    }

    /// Drops every read notification; returns how many.
    pub fn clear_read(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|n| n.unread);
        before - self.items.len()
    }

    /// Drops one read notification. An unread one is refused: it is still
    /// waiting on the user or on a transfer, so it goes through `mark_read` first.
    pub fn remove(&mut self, id: &str) -> bool {
        match self.items.iter().position(|n| n.id == id && !n.unread) {
            Some(i) => {
                self.items.remove(i);
                true
            }
            None => false,
        }
    }

    fn trim(&mut self) {
        while self.items.len() > MAX_NOTIFICATIONS {
            // Oldest read first, then the oldest unread that is not pinned.
            let victim = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, n)| !n.is_pinned())
                .min_by_key(|(_, n)| (n.unread, n.read_at_ms.unwrap_or(n.created_at_ms)))
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.items.remove(i);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(id: &str, kind: Kind, timeout_ms: u32) -> PromptIn {
        PromptIn {
            prompt_id: id.into(),
            kind,
            peer_name: "Pixel".into(),
            code: String::new(),
            content: String::new(),
            content_kind: "text".into(),
            timeout_ms,
        }
    }

    fn transfer(device: &str, file: &str, state: &str, done: u64) -> TransferIn {
        TransferIn {
            device_id: device.into(),
            device_name: device.to_uppercase(),
            file_id: file.into(),
            file_name: format!("{file}.bin"),
            direction: "download".into(),
            done,
            total: 100,
            state: state.into(),
            message: String::new(),
            can_pause: true,
        }
    }

    fn ids(list: &[Notification]) -> Vec<&str> {
        list.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn a_new_prompt_is_unread_and_waiting_with_its_deadline() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        let n = s.get("p1").unwrap();
        assert!(n.unread && n.is_waiting_prompt());
        assert_eq!(n.prompt.as_ref().unwrap().expires_at_ms, 31_000);
        assert_eq!((s.unread_count(), s.waiting_prompt_count()), (1, 1));
    }

    #[test]
    fn the_same_prompt_sent_again_is_not_duplicated_or_reset() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        s.on_prompt_answered("p1", "allowed", 2_000);
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 3_000);
        assert_eq!(s.snapshot().len(), 1);
        assert!(
            !s.get("p1").unwrap().unread,
            "a reconnect must not revive an answered prompt"
        );
    }

    #[test]
    fn answering_moves_a_prompt_to_read_with_its_outcome() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        assert!(s.on_prompt_answered("p1", "allowed", 2_000));
        let n = s.get("p1").unwrap();
        assert!(!n.unread);
        assert_eq!(n.read_at_ms, Some(2_000));
        let p = n.prompt.as_ref().unwrap();
        assert_eq!(
            (p.status, p.outcome.as_deref()),
            (PromptStatus::Answered, Some("allowed"))
        );
        assert!(
            !s.on_prompt_answered("p1", "denied", 3_000),
            "only the first answer counts"
        );
        assert!(!s.on_prompt_answered("nope", "allowed", 3_000));
    }

    #[test]
    fn a_timed_out_prompt_expires_into_read() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::IncomingClipboard, 5_000), 1_000);
        s.on_prompt_closed("p1", true, 6_000);
        let n = s.get("p1").unwrap();
        assert!(!n.unread);
        assert_eq!(n.prompt.as_ref().unwrap().status, PromptStatus::Expired);
    }

    #[test]
    fn a_prompt_answered_elsewhere_is_closed_and_the_local_answer_is_not_overwritten() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        s.on_prompt(prompt("p2", Kind::PairRequest, 30_000), 1_000);
        s.on_prompt_closed("p1", false, 2_000);
        assert_eq!(
            s.get("p1")
                .unwrap()
                .prompt
                .as_ref()
                .unwrap()
                .outcome
                .as_deref(),
            Some("handledElsewhere")
        );
        // The core's "closed" that follows a local answer changes nothing.
        s.on_prompt_answered("p2", "denied", 2_000);
        s.on_prompt_closed("p2", false, 2_001);
        assert_eq!(
            s.get("p2")
                .unwrap()
                .prompt
                .as_ref()
                .unwrap()
                .outcome
                .as_deref(),
            Some("denied")
        );
    }

    #[test]
    fn unread_is_oldest_first_and_read_is_newest_first() {
        let mut s = Store::new();
        s.on_prompt(prompt("a", Kind::PairRequest, 60_000), 1_000);
        s.on_prompt(prompt("b", Kind::PairRequest, 60_000), 2_000);
        s.on_prompt(prompt("c", Kind::PairRequest, 60_000), 3_000);
        s.on_prompt(prompt("d", Kind::PairRequest, 60_000), 4_000);
        s.on_prompt_answered("a", "allowed", 10_000);
        s.on_prompt_answered("c", "allowed", 20_000);
        assert_eq!(ids(&s.snapshot()), vec!["b", "d", "c", "a"]);
    }

    #[test]
    fn info_stays_unread_until_marked() {
        let mut s = Store::new();
        s.on_info("i1".into(), "Image saved".into(), "a.png".into(), 1_000);
        assert!(s.get("i1").unwrap().unread);
        assert!(s.mark_read("i1", 2_000));
        assert!(!s.get("i1").unwrap().unread);
    }

    #[test]
    fn a_waiting_prompt_and_a_running_transfer_cannot_be_marked_read() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        s.on_transfer(transfer("dev-a", "f1", "receiving", 10), 1_000);
        assert!(!s.mark_read("p1", 2_000));
        assert!(!s.mark_read(&transfer_id("dev-a", "f1"), 2_000));
        assert_eq!(s.mark_all_read(2_000), 0);
        assert_eq!(s.unread_count(), 2);
    }

    #[test]
    fn an_interrupted_transfer_can_be_dismissed_and_returns_when_it_restarts() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "interrupted", 40), 1_000);
        let id = transfer_id("dev-a", "f1");
        assert!(s.get(&id).unwrap().unread);
        assert!(
            s.mark_read(&id, 2_000),
            "nothing is running, so it can be dismissed"
        );
        s.on_transfer(transfer("dev-a", "f1", "receiving", 50), 3_000);
        assert!(
            s.get(&id).unwrap().unread,
            "it is unread again once it runs"
        );
    }

    #[test]
    fn a_transfer_is_one_notification_updated_in_place() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "requesting", 0), 1_000);
        s.on_transfer(transfer("dev-a", "f1", "receiving", 40), 2_000);
        s.on_transfer(transfer("dev-a", "f1", "receiving", 80), 3_000);
        let list = s.snapshot();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].created_at_ms, 1_000, "keeps its place in the order");
        assert_eq!(list[0].transfer.as_ref().unwrap().done, 80);
    }

    #[test]
    fn one_offer_to_two_devices_is_two_notifications() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "sending", 10), 1_000);
        s.on_transfer(transfer("dev-b", "f1", "sending", 50), 1_000);
        assert_eq!(s.snapshot().len(), 2);
        s.on_transfer(transfer("dev-a", "f1", "cancelled", 10), 2_000);
        assert!(!s.get(&transfer_id("dev-a", "f1")).unwrap().unread);
        assert!(
            s.get(&transfer_id("dev-b", "f1")).unwrap().unread,
            "the other device is unaffected"
        );
    }

    #[test]
    fn a_finished_transfer_stays_unread_until_seen_and_a_cancelled_one_is_read() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "receiving", 50), 1_000);
        s.on_transfer(transfer("dev-a", "f1", "complete", 100), 2_000);
        let id = transfer_id("dev-a", "f1");
        assert!(s.get(&id).unwrap().unread);
        assert!(
            s.mark_read(&id, 3_000),
            "a finished transfer can be marked read"
        );

        s.on_transfer(transfer("dev-a", "f2", "paused", 30), 4_000);
        s.on_transfer(transfer("dev-a", "f2", "cancelled", 30), 5_000);
        assert!(!s.get(&transfer_id("dev-a", "f2")).unwrap().unread);
    }

    #[test]
    fn a_new_attempt_on_a_read_transfer_makes_it_unread_again() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "cancelled", 30), 1_000);
        let id = transfer_id("dev-a", "f1");
        assert!(!s.get(&id).unwrap().unread);
        s.on_transfer(transfer("dev-a", "f1", "requesting", 0), 2_000);
        let n = s.get(&id).unwrap();
        assert!(n.unread && n.read_at_ms.is_none());
    }

    #[test]
    fn seeding_adds_only_missing_transfers() {
        let mut s = Store::new();
        s.on_transfer(transfer("dev-a", "f1", "receiving", 90), 1_000);
        s.seed_transfers(
            vec![
                transfer("dev-a", "f1", "receiving", 10),
                transfer("dev-b", "f2", "paused", 5),
            ],
            2_000,
        );
        assert_eq!(s.snapshot().len(), 2);
        assert_eq!(
            s.get(&transfer_id("dev-a", "f1"))
                .unwrap()
                .transfer
                .as_ref()
                .unwrap()
                .done,
            90,
            "a live value is not overwritten by an older snapshot"
        );
    }

    #[test]
    fn mark_all_read_and_clear_read() {
        let mut s = Store::new();
        s.on_info("i1".into(), "a".into(), "".into(), 1_000);
        s.on_info("i2".into(), "b".into(), "".into(), 1_000);
        s.on_prompt(prompt("p1", Kind::PairRequest, 30_000), 1_000);
        assert_eq!(
            s.mark_all_read(2_000),
            2,
            "the waiting prompt is left alone"
        );
        assert_eq!(s.unread_count(), 1);
        assert_eq!(s.clear_read(), 2);
        assert_eq!(ids(&s.snapshot()), vec!["p1"]);
    }

    #[test]
    fn remove_drops_one_read_notification_and_refuses_unread_ones() {
        let mut s = Store::new();
        s.on_info("i1".into(), "a".into(), "".into(), 1_000);
        s.on_info("i2".into(), "b".into(), "".into(), 1_000);
        s.mark_read("i1", 2_000);
        assert!(!s.remove("i2"), "unread stays until it is dealt with");
        assert!(!s.remove("nope"));
        assert!(s.remove("i1"));
        assert_eq!(ids(&s.snapshot()), vec!["i2"]);
    }

    #[test]
    fn the_list_is_capped_dropping_old_read_ones_but_never_pinned_ones() {
        let mut s = Store::new();
        s.on_prompt(prompt("waiting", Kind::PairRequest, 60_000), 1);
        s.on_transfer(transfer("dev-a", "running", "receiving", 1), 2);
        for i in 0..(MAX_NOTIFICATIONS + 30) {
            let id = format!("i{i}");
            s.on_info(id.clone(), "x".into(), "".into(), 10 + i as u64);
            s.mark_read(&id, 1_000 + i as u64);
        }
        let list = s.snapshot();
        assert_eq!(list.len(), MAX_NOTIFICATIONS);
        assert!(
            s.get("waiting").is_some(),
            "a waiting prompt is never dropped"
        );
        assert!(s.get(&transfer_id("dev-a", "running")).is_some());
        assert!(s.get("i0").is_none(), "the oldest read one went first");
        assert!(s.get(&format!("i{}", MAX_NOTIFICATIONS + 29)).is_some());
    }

    #[test]
    fn snapshot_serializes_for_the_front_end_in_camel_case() {
        let mut s = Store::new();
        s.on_prompt(prompt("p1", Kind::ConfirmCode, 30_000), 1_000);
        let json = serde_json::to_value(s.snapshot()).unwrap();
        assert_eq!(json[0]["kind"], "confirmCode");
        assert_eq!(json[0]["createdAtMs"], 1_000);
        assert_eq!(json[0]["prompt"]["status"], "waiting");
        assert_eq!(json[0]["prompt"]["peerName"], "Pixel");
    }
}
