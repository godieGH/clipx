use super::{platform::{NotificationEngine as Engine, NotificationFuture}, IncomingClipboardDecision, PairDecision, Prompt};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri_winrt_notification::Toast;
use tokio::sync::oneshot;

// TODO: swap for ClipX's own registered AUMID once the installer creates a
// Start Menu shortcut with an AppUserModelID set. Borrowing PowerShell's id
// works for development but ships the toast labeled "Windows PowerShell".
const APP_ID: &str = if cfg!(debug_assertions) {Toast::POWERSHELL_APP_ID } else {"com.godiegh.clipx"};

#[derive(Clone)]
pub struct NotificationEngine;

#[allow(unused)]
impl NotificationEngine {
    pub fn new() -> Self {
        Self
    }

        /// Shared implementation for all interactive Windows notifications.
    ///
    /// The platform-specific toast mechanics live here exactly once.
    /// Individual notification types only provide their buttons,
    /// action mapping, and fallback decision.
    async fn show_prompt<D, F>(
        &self,
        title: &str,
        body: &str,
        timeout: Duration,
        buttons: &[(&str, &str)],
        map_action: F,
        fallback: D,
    ) -> D
    where
        D: Send + Clone + 'static,
        F: Fn(Option<&str>) -> D + Send + Sync + 'static,
    {
        let (tx, rx) = oneshot::channel::<D>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        let tx_activated = Arc::clone(&tx);
        let tx_dismissed = Arc::clone(&tx);

        let title = title.to_string();
        let body = body.to_string();

        let map_action = Arc::new(map_action);
        let map_action_activated = Arc::clone(&map_action);

        // .show() performs synchronous COM/WinRT work.
        // block_in_place prevents that work from blocking other Tokio tasks
        // on this worker thread.
        let result = tokio::task::block_in_place(|| {
            let mut toast = Toast::new(APP_ID)
                .title(&title)
                .text1(&body)
                .duration(tauri_winrt_notification::Duration::Long);

            for &(label, action) in buttons {
                toast = toast.add_button(label, action);
            }

            toast
                .on_activated(move |action| {
                    let decision = map_action_activated(action.as_deref());

                    if let Some(sender) =
                        tx_activated.lock().unwrap().take()
                    {
                        let _ = sender.send(decision);
                    }

                    Ok(())
                })
                .on_dismissed(move |_reason| {
                    if let Some(sender) =
                        tx_dismissed.lock().unwrap().take()
                    {
                        let _ = sender.send(fallback.clone());
                    }

                    Ok(())
                })
                .show()
        });

        if let Err(e) = result {
            tracing::error!("failed to show toast: {e:?}");
            return map_action(None);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => decision,

            // Sender disappeared without producing a decision.
            Ok(Err(_)) => map_action(None),

            // We stopped waiting. The toast may still be visible.
            Err(_) => map_action(None),
        }
    }
}

impl Engine for NotificationEngine {
    fn ask_pair<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, PairDecision> {
        Box::pin(async move {
            let (title, body) = prompt.render();

            self.show_prompt(
                &title,
                &body,
                timeout,
                &[("Allow", "allow"), ("Deny", "deny")],
                |action| match action {
                    Some("allow") => PairDecision::Allow,
                    Some("deny") => PairDecision::Deny,
                    _ => PairDecision::NoResponse,
                },
                PairDecision::NoResponse,
            )
            .await
        })
    }

    fn ask_clipboard<'a>(
        &'a self,
        prompt: Prompt,
        timeout: Duration,
    ) -> NotificationFuture<'a, IncomingClipboardDecision> {
        Box::pin(async move {
            let (title, body) = prompt.render();

            self.show_prompt(
                &title,
                &body,
                timeout,
                &[("Copy to clipboard", "copy")],
                |action| match action {
                    Some("copy") => IncomingClipboardDecision::Copy,
                    _ => IncomingClipboardDecision::Ignore,
                },
                IncomingClipboardDecision::Ignore,
            )
            .await
        })
    }

    fn notify_info<'a>(
        &'a self,
        title: &'a str,
        body: String,
    ) -> NotificationFuture<'a, ()> {
        Box::pin(async move {
            let title = title.to_string();

            tokio::task::block_in_place(|| {
                let _ = Toast::new(APP_ID)
                    .title(&title)
                    .text1(&body)
                    .show();
            });
        })
    }
}

impl Default for NotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}