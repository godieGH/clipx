//use std::collections::HashMap;
use tokio::net::TcpListener;
//use tokio_tungstenite::accept_async;
//use futures_util::{SinkExt, Stream};

pub struct Transport {
    listener: TcpListener,
    //connections: HashMap<DeviceId, WsConnection>,
}

impl Transport {
    pub async fn create_transport() -> Self {
        let ws_port = crate::device::config::get_ws_port();
        let listener = TcpListener::bind(format!("0.0.0.0:{ws_port}"))
            .await
            .unwrap();
        tracing::info!("WS server running on port={ws_port} ...");
        Self { listener }
    }
}
