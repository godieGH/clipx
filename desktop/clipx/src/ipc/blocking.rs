use crate::clipx;
use interprocess::local_socket::{prelude::*, GenericNamespaced, Name, Stream, ToNsName};
use prost::Message;
use tungstenite::{client, WebSocket};

pub struct Client {
    name: String,
    ws: Option<WebSocket<Stream>>,
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

    pub fn start(mut self) -> Self {
        if self.is_running() {
            println!("The Ipc service is already running...");
            return self;
        };
        let name = self.create_name().unwrap();
        let conn = Stream::connect(name).unwrap();
        let (ws, _response) = client("ws://localhost/", conn).unwrap();
        self.ws = Some(ws);
        self.state = State::Running;
        self
    }

    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Sends one request, reads one response. Connection stays open for reuse.
    pub fn send(&mut self, _msg: impl prost::Message) -> anyhow::Result<clipx::IpcResponse> {
        if !self.is_running() {
            return Err(anyhow::anyhow!("The Ipc service is down"));
        };
        let mut buf = Vec::new();
        _msg.encode(&mut buf)?;

        let ws = self.ws.as_mut().unwrap();
        if let Err(e) = ws.send(tungstenite::Message::binary(buf)) {
            self.state = State::Down;
            self.ws = None;
            return Err(e.into());
        }

        loop {
            let msg = ws.read();
            match msg {
                Ok(tungstenite::Message::Binary(bytes)) => {
                    let envelope = clipx::IpcServerMessage::decode(bytes.as_slice())?;
                    match envelope.payload {
                        Some(clipx::ipc_server_message::Payload::Response(resp)) => {
                            return Ok(resp);
                        }
                        // One-shot CLI, doesn't subscribe to push events —
                        // skip and keep waiting for the actual reply.
                        Some(clipx::ipc_server_message::Payload::Event(_)) | None => continue,
                    }
                }
                Ok(tungstenite::Message::Close(_)) => {
                    self.state = State::Down;
                    self.ws = None;
                    break;
                }
                Ok(_) => continue,
                Err(_) => {
                    self.state = State::Down;
                    self.ws = None;
                    break;
                }
            }
        }
        Err(anyhow::anyhow!("No response"))
    }

    /// Explicit teardown — call once, when the CLI is done issuing commands
    /// (e.g. at the end of `main`, or when a long-running subcommand exits).
    pub fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(ws) = self.ws.as_mut() {
            ws.close(None)?;
            loop {
                match ws.read() {
                    Ok(tungstenite::Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        }
        self.ws = None;
        self.state = State::Down;
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(ws) = self.ws.as_mut() {
            let _ = ws.close(None);
            loop {
                match ws.read() {
                    Ok(tungstenite::Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        }
        self.ws = None;
        self.state = State::Down;
    }
}
