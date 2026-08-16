use super::{PairDecision, Prompt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri_winrt_notification::Toast;
use tokio::sync::oneshot;

// TODO: swap for ClipX's own registered AUMID once the installer creates a
// Start Menu shortcut with an AppUserModelID set. Borrowing PowerShell's id
// works for development but ships the toast labeled "Windows PowerShell".
const APP_ID: &str = Toast::POWERSHELL_APP_ID;

#[derive(Clone)]
pub struct NotificationEngine;

#[allow(unused)]
impl NotificationEngine {
    pub fn new() -> Self {
        Self
    }

    /// Shows the appropriate native prompt for `prompt` and waits for a
    /// decision, a dismissal, or `timeout` — whichever comes first.
    pub async fn ask(&self, prompt: Prompt, timeout: Duration) -> PairDecision {
        let (title, body) = render(&prompt);
        self.show_allow_deny(&title, &body, timeout).await
    }

    async fn show_allow_deny(&self, title: &str, body: &str, timeout: Duration) -> PairDecision {
        let (tx, rx) = oneshot::channel::<PairDecision>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        let tx_activated = tx.clone();
        let tx_dismissed = tx.clone();
        let title = title.to_string();
        let body = body.to_string();

        // .show() does real synchronous COM/WinRT work — a few ms, but still
        // blocking. block_in_place tells tokio to hand this worker's other
        // queued tasks to another thread for that window, so nothing else
        // on this worker starves. Only valid on the multi-thread runtime.
        let result = tokio::task::block_in_place(|| {
            Toast::new(APP_ID)
                .title(&title)
                .text1(&body)
                .add_button("Allow", "allow")
                .add_button("Deny", "deny")
                .duration(tauri_winrt_notification::Duration::Long)
                .on_activated(move |action| {
                    let decision = match action.as_deref() {
                        Some("allow") => PairDecision::Allow,
                        Some("deny") => PairDecision::Deny,
                        _ => PairDecision::NoResponse,
                    };
                    if let Some(sender) = tx_activated.lock().unwrap().take() {
                        let _ = sender.send(decision);
                    }
                    Ok(())
                })
                .on_dismissed(move |_reason| {
                    if let Some(sender) = tx_dismissed.lock().unwrap().take() {
                        let _ = sender.send(PairDecision::NoResponse);
                    }
                    Ok(())
                })
                .show()
        });

        if let Err(e) = result {
            tracing::error!("failed to show toast: {e:?}");
            return PairDecision::NoResponse;
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => PairDecision::NoResponse, // sender dropped without sending
            Err(_) => PairDecision::NoResponse,     // stopped waiting; toast may still be on screen
        }
    }
}

fn render(prompt: &Prompt) -> (String, String) {
    match prompt {
        Prompt::PairRequest { peer_name } => (
            "Pairing request".to_string(),
            format!("{peer_name} wants to pair with this device."),
        ),
        Prompt::ConfirmCode { peer_name, code } => (
            "Confirm pairing code".to_string(),
            format!("Code from {peer_name}: {code}\nDoes this match on both devices?"),
        ),
    }
}