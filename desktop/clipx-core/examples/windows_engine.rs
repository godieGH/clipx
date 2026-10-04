use clipx_core::{
    logging,
    notification::{NotificationEngine, platform::NotificationEngine as _},
};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let _guard = logging::init();
    let engine = NotificationEngine::new();

    let id = "test-file-1".to_string();
    let name = "movie.mkv".to_string();
    let total: u64 = 100_000_000;
    let step_ms: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let send = |done: u64, state: &str, msg: &str| {
        engine.notify_file_transfer(
            id.clone(),
            name.clone(),
            "download".into(),
            done,
            total,
            state.into(),
            msg.into(),
            None,
        )
    };

    send(0, "requesting", "Downloading file").await;
    for pct in 1..=100u64 {
        tokio::time::sleep(Duration::from_millis(step_ms)).await;
        send(total * pct / 100, "receiving", "Downloading").await;
    }
    send(total, "saving", "Saving movie.mkv").await;
    send(total, "complete", r"C:\Users\Admin\Downloads\movie.mkv").await;

    tokio::time::sleep(Duration::from_secs(8)).await;
}
