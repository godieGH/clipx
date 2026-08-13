//! This is non blocking implementation of the blocking IPC
//! It does nothing much but to wrap all blocking ipc APIs so the can be used asyncrounous

use crate::clipx;
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{
    tokio::Stream as LocalStream, ConnectOptions, GenericNamespaced, Name, ToNsName,
};
use prost::Message;
use tokio_tungstenite::{client_async, tungstenite::Message as WsMessage, WebSocketStream};

/// Same Client as the blocking one but this only holds a differerent Non blocking ws 
pub struct Client {
    name: String,
    ws: Option<WebSocketStream<LocalStream>>,
    state: State,
}

enum State {
    Running,
    Down,
}

/// Implements same APIs but with slightly different signature to allow async code to call safely
impl Client {
    /// This create the non-blocking Client instance
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ws: None,
            state: State::Down,
        }
    }

    /// This runs the non-blocking Client
    /// **Note:** This doesn't consume and return new ipc instance it just modifies and starts the ws server so no need to call `.new().run()` and re-assign as on blocking
    pub async fn run(&mut self) -> Result<(), String> {
        if let State::Running = self.state {
            println!("The Ipc service is already running...");
            return Ok(());
        };
        let name = self.create_name().unwrap();
        let conn = ConnectOptions::new()
            .name(name)
            .connect_tokio()
            .await
            .map_err(|e| format!("Failed to connect to the IPC: {}", e.to_string()))?;

        let (ws, _response) = client_async("ws://localhost/", conn)
            .await
            .map_err(|e| format!("Websocket handshake failed: {}", e.to_string()))?;
        self.ws = Some(ws);

        self.state = State::Running;
        Ok(())
    }

    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    /// helper to see if the server is running useful to call `.run()` safely 
    pub fn is_running(&self) -> bool {
        match self.state {
            State::Running => true,
            State::Down => false
        }
    }

    /// works same as the blocking one but this yields so it is non-blocking
    pub async fn send(&mut self, _msg: impl prost::Message) -> anyhow::Result<clipx::IpcResponse> {
        if let State::Down = self.state {
            println!("The Ipc service is down");
            return Err(anyhow::anyhow!("The Ipc service is down"));
        };
        let mut buf = Vec::new();
        _msg.encode(&mut buf)?;

        let ws = self.ws.as_mut().unwrap();
        ws.send(WsMessage::binary(buf)).await?;

        while let Some(msg) = ws.next().await {
            match msg {
                Ok(msg) => match msg {
                    WsMessage::Binary(bytes) => {
                        let res = clipx::IpcResponse::decode(bytes)?;
                        ws.close(None).await?;
                        while let Some(msg) = ws.next().await {
                            match msg {
                                Ok(WsMessage::Close(_)) => {
                                    break;
                                }
                                Ok(_) => continue,
                                Err(_) => break,
                            }
                        }
                        return Ok(res);
                    }
                    WsMessage::Close(_) => {
                        break;
                    }
                    _ => {}
                },
                Err(_) => {
                    break;
                }
            }
        }
        Err(anyhow::anyhow!("No response"))
    }
}
