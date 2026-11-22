use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Text,
    RichText,
    Image,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipItem {
    pub id: String,
    pub content: String,
    pub source_device: String,
    #[serde(default)]
    pub source_device_id: String,
    pub received_at_ms: u64,
    #[serde(default = "default_kind")]
    pub kind: ClipKind,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub file_expires_at_ms: u64,
    #[serde(default)]
    pub file_downloaded: bool,
    #[serde(default)]
    pub local_file_path: Option<String>,
}

fn default_kind() -> ClipKind {
    ClipKind::Text
}

pub struct ClipboardStore {
    path: PathBuf,
    pub history: VecDeque<ClipItem>,
    max_capacity: usize,
}

impl ClipboardStore {
    pub fn load(path: PathBuf, max_capacity: usize) -> Self {
        let history: VecDeque<ClipItem> = match fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<VecDeque<ClipItem>>(&data) {
                Ok(history) => {
                    tracing::debug!(entries = history.len(), "clipboard history loaded");
                    history
                }
                Err(error) => {
                    tracing::warn!(%error, "clipboard history is invalid; starting empty");
                    VecDeque::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::trace!("clipboard history does not exist yet");
                VecDeque::new()
            }
            Err(error) => {
                tracing::warn!(%error, "could not read clipboard history; starting empty");
                VecDeque::new()
            }
        };
        Self {
            path,
            history,
            max_capacity,
        }
    }

    pub fn add(&mut self, item: ClipItem) -> bool {
        if self
            .history
            .front()
            .is_some_and(|top| same_clip(top, &item))
        {
            tracing::trace!(entry_id = %item.id, "duplicate clipboard history head ignored");
            return false;
        }
        let entry_id = item.id.clone();
        let kind = item.kind.clone();
        self.history.push_front(item);
        while self.history.len() > self.max_capacity {
            self.history.pop_back();
        }
        self.persist();
        tracing::trace!(%entry_id, ?kind, entries = self.history.len(), "clipboard history entry added");
        true
    }

    pub fn update(&mut self, item: ClipItem) -> bool {
        let Some(pos) = self.history.iter().position(|v| v.id == item.id) else {
            return false;
        };
        let id = item.id.clone();
        self.history[pos] = item;
        self.persist();
        tracing::trace!(%id, "clipboard history entry updated");
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.history.len();
        self.history.retain(|i| i.id != id);
        let removed = self.history.len() != before;
        if removed {
            self.persist();
            tracing::trace!(%id, "clipboard history entry removed");
        }
        removed
    }

    pub fn clear(&mut self) {
        let entries = self.history.len();
        self.history.clear();
        self.persist();
        tracing::debug!(entries, "clipboard history cleared");
    }

    pub(super) fn persist(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.history) {
            if let Err(e) = fs::write(&self.path, json) {
                tracing::error!(error = %e, "failed to persist clipboard history");
            } else {
                tracing::trace!(entries = self.history.len(), "clipboard history persisted");
            }
        } else {
            tracing::error!("failed to serialize clipboard history");
        }
    }
}

fn same_clip(a: &ClipItem, b: &ClipItem) -> bool {
    a.kind == b.kind && a.content == b.content && a.html == b.html && a.file_id == b.file_id
}
