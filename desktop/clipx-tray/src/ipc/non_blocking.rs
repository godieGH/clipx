//! This is non blocking implementation of the blocking IPC
//! It does nothing much but to wrap all blocking ipc APIs so the can be used asyncrounous

use crate::clipx;
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{
    tokio::Stream as LocalStream, ConnectOptions, GenericNamespaced, Name, ToNsName,
};
use prost::Message;
use tokio_tungstenite::{client_async, tungstenite::Message as WsMessage, WebSocketStream};

/// Same Client as the blocking one but this only holds a different non-blocking ws
pub struct Client {
    name: String,
    ws: Option<WebSocketStream<LocalStream>>,
    state: State,
}

enum State {
    Running,
    Down,
}

impl Client {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ws: None,
            state: State::Down,
        }
    }

    /// Opens the persistent IPC + WS connection. Safe to call again if already running (no-op).
    pub async fn start(&mut self) -> Result<(), String> {
        if self.is_running() {
            println!("The Ipc service is already running...");
            return Ok(());
        };
        let name = self.create_name().unwrap();
        let conn = ConnectOptions::new()
            .name(name)
            .connect_tokio()
            .await
            .map_err(|e| format!("Failed to connect to the IPC: {}", e))?;

        let (ws, _response) = client_async("ws://localhost/", conn)
            .await
            .map_err(|e| format!("Websocket handshake failed: {}", e))?;
        self.ws = Some(ws);
        self.state = State::Running;
        Ok(())
    }

    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Sends one request and reads one response over the shared persistent connection.
    /// Does NOT close the socket — the connection stays open for reuse across calls.
    pub async fn send(&mut self, _msg: impl prost::Message) -> anyhow::Result<clipx::IpcResponse> {
        if !self.is_running() {
            return Err(anyhow::anyhow!("The Ipc service is down"));
        }
        let mut buf = Vec::new();
        _msg.encode(&mut buf)?;

        let ws = self.ws.as_mut().unwrap();
        if let Err(e) = ws.send(WsMessage::binary(buf)).await {
            // the write itself failed — connection is dead mark state to down 
            self.state = State::Down;
            self.ws = None;
            return Err(e.into());
        };

        while let Some(msg) = ws.next().await {
            match msg {
                Ok(WsMessage::Binary(bytes)) => {
                    return Ok(clipx::IpcResponse::decode(bytes)?);
                }
                Ok(WsMessage::Close(_)) => {
                    // Peer closes connection without a response Mark state to down
                    // and break the loop so send returns a no response
                    self.state =  State::Down;
                    self.ws = None;
                    break;
                }
                Ok(_) => continue, // ping/pong/text — ignore, keep waiting
                Err(_) => {
                    // Connection is broken, not just quiet. Mark down so callers
                    // (and is_running()) know to reconnect rather than retry blindly.
                    self.state = State::Down;
                    self.ws = None;
                    break;
                }
            }
        }
        Err(anyhow::anyhow!("No response"))
    }

    /// Explicit, one-time teardown of the shared connection. Call this once, on app exit —
    /// not per-request, and not per-window.
    pub async fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(mut ws) = self.ws.take() {
            ws.close(None).await?;
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(WsMessage::Close(_)) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        }
        self.state = State::Down;
        Ok(())
    }
}