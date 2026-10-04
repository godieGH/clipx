//! This is non blocking implementation of the blocking IPC
//! It does nothing much but to wrap all blocking ipc APIs so the can be used asyncrounous

use crate::clipx;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{
    tokio::Stream as LocalStream, ConnectOptions, GenericNamespaced, Name, ToNsName,
};
use prost::Message;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::{client_async, tungstenite::Message as WsMessage, WebSocketStream};

type WsSink = SplitSink<WebSocketStream<LocalStream>, WsMessage>;

/// Same Client as the blocking one but this only holds a different non-blocking ws.
///
/// The socket is full-duplex and the core can now push unsolicited `IpcEvent`
/// frames at any time (see clipx.proto's `IpcServerMessage`), not just replies
/// to whatever was last sent. So the read half is owned by a dedicated
/// background task, not by `send()` — `send()` only ever writes, then awaits
/// a oneshot that the reader task fills in once *the* matching response frame
/// (not an event) comes back. Only one request is ever in flight at a time
/// today (every call site awaits `send()` before issuing another), so a
/// single pending-slot is enough — no per-request IDs needed.
pub struct Client {
    name: String,
    state: State,
    sink: Option<Arc<Mutex<WsSink>>>,
    pending: Arc<Mutex<Option<oneshot::Sender<clipx::IpcResponse>>>>,
    /// Unsolicited pushes land here. Taken once (typically at app startup)
    /// by whoever wants to bridge them onward — e.g. Tauri's event emitter.
    events_rx: Option<mpsc::UnboundedReceiver<clipx::IpcEvent>>,
    events_tx: mpsc::UnboundedSender<clipx::IpcEvent>,
    reader_task: Option<JoinHandle<()>>,
}

enum State {
    Running,
    Down,
}

impl Client {
    pub fn new(name: impl Into<String>) -> Self {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        Self {
            name: name.into(),
            state: State::Down,
            sink: None,
            pending: Arc::new(Mutex::new(None)),
            events_rx: Some(events_rx),
            events_tx,
            reader_task: None,
        }
    }

    /// Takes the event receiver. Only meaningful once, right after
    /// construction — later calls return `None`. Whoever holds this is
    /// responsible for forwarding events onward (e.g. into a Tauri emit).
    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<clipx::IpcEvent>> {
        self.events_rx.take()
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

        let (sink, mut stream) = ws.split();
        self.sink = Some(Arc::new(Mutex::new(sink)));

        let pending = self.pending.clone();
        let events_tx = self.events_tx.clone();
        self.reader_task = Some(tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(WsMessage::Binary(bytes)) => {
                        let envelope = match clipx::IpcServerMessage::decode(bytes.as_ref()) {
                            Ok(env) => env,
                            Err(e) => {
                                eprintln!("Failed to decode IPC server message: {e}");
                                continue;
                            }
                        };
                        match envelope.payload {
                            Some(clipx::ipc_server_message::Payload::Response(resp)) => {
                                if let Some(tx) = pending.lock().await.take() {
                                    let _ = tx.send(resp);
                                }
                                // If nothing was waiting, the caller that
                                // wanted this already gave up — drop it.
                            }
                            Some(clipx::ipc_server_message::Payload::Event(event)) => {
                                let _ = events_tx.send(event);
                            }
                            None => {}
                        }
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    Ok(_) => continue, // ping/pong/text — ignore
                }
            }
            // Reader has ended — drop the pending slot so any in-flight
            // send() sees its oneshot canceled and surfaces "No response"
            // instead of hanging forever.
            pending.lock().await.take();
        }));

        self.state = State::Running;
        Ok(())
    }

    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Sends one request and awaits *the* response to it over the shared
    /// persistent connection. Unaffected by any push events that arrive
    /// while this is in flight — the reader task routes those elsewhere.
    /// Does NOT close the socket — the connection stays open for reuse.
    pub async fn send(&mut self, _msg: impl prost::Message) -> anyhow::Result<clipx::IpcResponse> {
        if !self.is_running() {
            return Err(anyhow::anyhow!("The Ipc service is down"));
        }
        let Some(sink) = self.sink.clone() else {
            return Err(anyhow::anyhow!("The Ipc service is down"));
        };

        let mut buf = Vec::new();
        _msg.encode(&mut buf)?;

        let (resp_tx, resp_rx) = oneshot::channel();
        *self.pending.lock().await = Some(resp_tx);

        if let Err(e) = sink.lock().await.send(WsMessage::binary(buf)).await {
            self.state = State::Down;
            self.sink = None;
            self.pending.lock().await.take();
            return Err(e.into());
        }

        match resp_rx.await {
            Ok(resp) => Ok(resp),
            Err(_) => {
                // Reader task dropped the sender — connection died mid-flight.
                self.state = State::Down;
                self.sink = None;
                Err(anyhow::anyhow!("No response"))
            }
        }
    }

    /// Explicit, one-time teardown of the shared connection. Call this once, on app exit —
    /// not per-request, and not per-window.
    pub async fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(sink) = self.sink.take() {
            let _ = sink.lock().await.close().await;
        }
        if let Some(task) = self.reader_task.take() {
            let _ = task.await;
        }
        self.state = State::Down;
        Ok(())
    }

    /// True only if the reader task is still alive, meaning the core is really there.
    pub fn is_connected(&self) -> bool {
        self.is_running() && self.reader_task.as_ref().is_some_and(|t| !t.is_finished())
    }

    pub async fn request_shutdown(&mut self) -> anyhow::Result<()> {
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Shutdown(
                clipx::ShutdownRequest {},
            )),
        };
        match self.send(req).await?.response {
            Some(clipx::ipc_response::Response::Shutdown(_)) => Ok(()),
            _ => Err(anyhow::anyhow!("unexpected response to shutdown")),
        }
    }
}
