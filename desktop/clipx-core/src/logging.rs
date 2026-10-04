use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{EnvFilter, fmt};

/// Keep the returned guard alive for the whole process, or the tail of the log is lost.
pub fn init() -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // dev: plain console output
    if cfg!(debug_assertions) {
        fmt().with_env_filter(filter).init();
        return None;
    }

    // release: rotating file, 7 days kept
    let dir = directories::ProjectDirs::from("com", "godiegh", "clipx")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("clipx-logs"));
    let _ = std::fs::create_dir_all(&dir);

    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("clipx-core")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir);

    let Ok(appender) = appender else {
        fmt().with_env_filter(filter).init(); // never crash because logging failed
        return None;
    };

    let (writer, guard) = tracing_appender::non_blocking(appender);
    fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();

    // panics bypass tracing, so route them into the log too
    std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));
    Some(guard)
}
