use std::path::PathBuf;

use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    EnvFilter, fmt,
    layer::{Layer, SubscriberExt},
    util::SubscriberInitExt,
};

pub fn log_directory() -> PathBuf {
    directories::ProjectDirs::from("com", "godiegh", "clipx")
        .map(|dirs| dirs.data_local_dir().join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("clipx-logs"))
}

/// Keep a rolling file in debug and release builds; debug also logs to stderr.
pub fn init() -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let dir = log_directory();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("ClipX: could not create log directory {}: {error}", dir.display());
        fmt().with_env_filter(filter).init();
        return None;
    }
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("clipx-core")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir);
    let Ok(appender) = appender else {
        fmt().with_env_filter(filter).init();
        return None;
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    if cfg!(debug_assertions) {
        tracing_subscriber::registry()
            .with(fmt::layer().with_filter(filter.clone()))
            .with(fmt::layer().with_writer(writer).with_ansi(false).with_filter(filter))
            .init();
    } else {
        fmt().with_env_filter(filter).with_writer(writer).with_ansi(false).init();
    }
    std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));
    Some(guard)
}
