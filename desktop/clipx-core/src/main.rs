use clipx_core::{service::CoreService, logging};

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
