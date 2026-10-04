#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clipx_core::{logging, service::CoreService};
use interprocess::local_socket::{ConnectOptions, GenericNamespaced, ToNsName};

async fn already_running() -> bool {
    match "clipx".to_ns_name::<GenericNamespaced>() {
        Ok(name) => ConnectOptions::new().name(name).connect_tokio().await.is_ok(),
        Err(_) => false,
    }
}

#[tokio::main]
async fn main() {
    let log_guard = logging::init();

    if already_running().await {
        tracing::warn!("clipx-core already running, exiting");
        return;
    }

    let service = CoreService::new();
    match service.run().await {
        Ok(()) => tracing::info!("Core exited cleanly"),
        Err(err) => {
            tracing::error!(error = %err, "Core service failed");
            drop(log_guard);
            std::process::exit(1);
        }
    }
}
