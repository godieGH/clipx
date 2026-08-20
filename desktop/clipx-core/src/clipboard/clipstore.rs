use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipItem {
    pub id: String,
    // The real clip content — can hold text data here or url/ref
    // later for other kind of clipboard ie. files, image.
    // Plain text and richtext are going to be held here
    pub content: String,
    // later for carrying raw image bytes
    // pub image: Vec<u8>,
    /// Display name of the device this entry arrived from. History only
    /// ever holds inbound content — what you copied locally never lands
    /// here, so this is never "local" or the user's own device.
    pub source_device: String,
    pub received_at_ms: u64,
}

/// Pure persistence for clipboard history. Knows nothing about the OS
/// clipboard, transports, or notifications — just a capped, ordered list
/// on disk (most-recent-first). Dedup-against-echo and any "should we ask
/// the user first" logic live in ClipboardManager, not here.
pub struct ClipboardStore {
    path: PathBuf,
    pub history: VecDeque<ClipItem>,
    max_capacity: usize,
}

impl ClipboardStore {
    pub fn load(path: PathBuf, max_capacity: usize) -> Self {
        let history = fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();
        Self {
            path,
            history,
            max_capacity,
        }
    }

    /// Pushes to the front (most recent first). Skips exact-duplicate
    /// back-to-back entries — e.g. the same URL copied twice in a row —
    /// then persists to disk.
   pub fn add(&mut self, item: ClipItem) -> bool {
        if self
            .history
            .front()
            .is_some_and(|top| top.content == item.content)
        {
            return false;
        }
        self.history.push_front(item);
        while self.history.len() > self.max_capacity {
            self.history.pop_back();
        }
        self.persist();
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.history.len();
        self.history.retain(|i| i.id != id);
        let removed = self.history.len() != before;
        if removed {
            self.persist();
        }
        removed
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.persist();
    }

    fn persist(&self) {
        match serde_json::to_string_pretty(&self.history) {
            Ok(json) => {
                if let Err(e) = fs::write(&self.path, json) {
                    tracing::error!("failed to persist clipboard history: {e}");
                }
            }
            Err(e) => tracing::error!("failed to serialize clipboard history: {e}"),
        }
    }
}
