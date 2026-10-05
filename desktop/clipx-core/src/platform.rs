//! Platform-facing contracts used by core to speak with the host operating system.
//!
//! The host implementation decides how to access the local clipboard, show
//! prompts, and route lifecycle events back into the app shell.

/// Platform implements this to give core an object that writes to the system clipboard.
pub trait ClipboardSink: Send + Sync {
    /// The general-wrapper around writing textual content to the sink
    fn write(&self, content: String) {
        self.write_text(content);
    }
    fn write_text(&self, content: String);
    fn write_rich_text(&self, content: String, _html: String) {
        self.write_text(content);
    }
    /// This writes an image clipboard given in as `height` and `rgba` vector
    fn write_image(&self, width: u32, height: u32, rgba: Vec<u8>);
    /// Save a user-approved downloaded file and return the user-visible path/URI.
    fn save_file(&self, name: &str, mime_type: &str, data: &[u8]) -> Result<String, String>;
    /// Save a staged file without materializing it into RAM.
    fn save_file_from_path(
        &self,
        name: &str,
        mime_type: &str,
        source: &std::path::Path,
    ) -> Result<String, String> {
        let data = std::fs::read(source).map_err(|e| e.to_string())?;
        self.save_file(name, mime_type, &data)
    }
}

/// Example of a desktop's clipboard Sink wrapper around the arboard crate
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub struct ArboardClipboardSink;

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl ArboardClipboardSink {
    /// Initialize the Sink
    pub fn new() -> Self {
        Self
    }
}
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl Default for ArboardClipboardSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl ClipboardSink for ArboardClipboardSink {
    fn write_text(&self, content: String) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(error) = clipboard.set_text(content) {
                    tracing::warn!(%error, "failed to apply remote clipboard text");
                }
            }
            Err(error) => tracing::warn!(%error, "failed to open clipboard to apply remote text"),
        }
    }
    fn write_image(&self, width: u32, height: u32, rgba: Vec<u8>) {
        let image = arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(rgba),
        };
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(error) = clipboard.set_image(image) {
                    tracing::warn!(%error, "failed to apply remote clipboard image");
                }
            }
            Err(error) => tracing::warn!(%error, "failed to open clipboard to apply remote image"),
        }
    }
    fn save_file(&self, name: &str, _mime_type: &str, data: &[u8]) -> Result<String, String> {
        let base = dirs_fallback();
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let mut path = base.join(safe_name(name));
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mut n = 1;
        while path.exists() {
            let filename = if ext.is_empty() {
                format!("{stem} ({n})")
            } else {
                format!("{stem} ({n}).{ext}")
            };
            path = base.join(filename);
            n += 1;
        }
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }

    fn save_file_from_path(
        &self,
        name: &str,
        _mime_type: &str,
        source: &std::path::Path,
    ) -> Result<String, String> {
        let base = dirs_fallback();
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let safe = safe_name(name);
        let mut path = base.join(&safe);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mut n = 1u32;
        while path.exists() {
            let filename = if ext.is_empty() {
                format!("{stem} ({n})")
            } else {
                format!("{stem} ({n}).{ext}")
            };
            path = base.join(filename);
            n += 1;
        }
        std::fs::copy(source, &path).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }
}

/// Helper to determine where the downloads directory resides
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
fn dirs_fallback() -> std::path::PathBuf {
    if let Ok(v) = std::env::var("XDG_DOWNLOAD_DIR") {
        return std::path::PathBuf::from(v).join("Clipx");
    }
    if let Ok(v) = std::env::var("USERPROFILE") {
        return std::path::PathBuf::from(v).join("Downloads").join("Clipx");
    }
    if let Ok(v) = std::env::var("HOME") {
        return std::path::PathBuf::from(v).join("Downloads").join("Clipx");
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".").join("Clipx"))
}

/// Helper that normalize file names to safe conventions
#[allow(unused)]
fn safe_name(name: &str) -> String {
    let candidate = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("clipx-file");
    candidate
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// platforms implements this to give core a way to prompt out or surface pop notifications
pub trait NotificationPrompter: Send + Sync {
    fn show_pair_request(&self, prompt_id: String, peer_name: String);
    fn show_pair_code(&self, prompt_id: String, peer_name: String, code: String);
    fn show_received_clipboard(&self, prompt_id: String, peer_name: String, action: String);
    fn notify_info(&self, title: String, message: String);
}

/// The core state changes or activities happen — core emits events
/// platform registers to the events so they sync/communicate with the core
#[derive(Debug, Clone)]
pub enum CoreEvent {
    DevicesChanged,
    ClipboardChanged,
    PairingChanged {
        device_id: String,
        state: PairingEventState,
        message: String,
    },
    FileTransferChanged {
        entry_id: String,
        file_id: String,
        file_name: String,
        direction: String,
        done: u64,
        total: u64,
        state: String,
        message: String,
    },
}

/// Another event type but for platform pairing updates
#[derive(Debug, Clone, Copy)]
pub enum PairingEventState {
    Started,
    Failed,
    Succeeded,
}

/// Platforms implement to register callers that the core calls into when events happen
pub trait CoreEventListener: Send + Sync {
    fn on_device_change(&self);
    fn on_clipboard_change(&self);
    fn on_pairing_change(&self, _device_id: String, _state: u8, _message: String) {}
    fn on_file_transfer(
        &self,
        _entry_id: String,
        _file_id: String,
        _file_name: String,
        _direction: String,
        _done: u64,
        _total: u64,
        _state: String,
        _message: String,
    ) {
    }
}
