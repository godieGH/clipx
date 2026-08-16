mod clipboard;
mod device;
mod logging;
mod message;
mod net;
mod service;
mod notification;

use service::CoreService;

#[tokio::main]
async fn main() {
    logging::init();

    let service = CoreService::new();
    match service.run().await {
        Ok(()) => tracing::info!("Core exited cleanly"),
        Err(err) => {
            tracing::error!(error = %err, "Core service failed");
            std::process::exit(1);
        }
    }
}
