use std::time::Duration;

use clipx_core::notification::{IncomingClipboardDecision, NotificationEngine, Prompt};

#[tokio::main]
async fn main() {

    let a = NotificationEngine;

    let b = a.ask_clipboard(
        Prompt::IncomingClipboard {
            peer_name: String::from("Joan PC"),
            content: String::default()
        },
        Duration::from_secs(10)
    ).await;

    match b {
      IncomingClipboardDecision::Copy => {
        println!("Copied")
      }
      IncomingClipboardDecision::Ignore => {
        println!("Ignored")
      }
    }
}