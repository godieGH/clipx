use interprocess::local_socket::{GenericNamespaced, Name, Stream, prelude::*, ToNsName};
use tungstenite::{client, WebSocket};
use crate::clipx;
use prost::Message;

pub struct IpcClient {
    name: String,
    ws: Option<WebSocket<Stream>>,
    state: State,
}

enum State {
    Running,
    Down,
}

impl IpcClient {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ws: None,
            state: State::Down
        }
    }

    pub fn run(mut self) -> Self {
        if let State::Running = self.state {
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

    pub fn send(&mut self, _msg: impl prost::Message) -> anyhow::Result<clipx::IpcResponse> {
        if let State::Down = self.state {
            println!("The Ipc service is down");
            return Err(anyhow::anyhow!("The Ipc service is down"))
        };
        let mut buf = Vec::new();
        _msg.encode(&mut buf)?;

        let ws = self.ws.as_mut().unwrap();
        ws.send(tungstenite::Message::binary(buf))?;

        loop {
            let msg = ws.read()?;

            match msg {
                tungstenite::Message::Binary(bytes) => {
                    let res = clipx::IpcResponse::decode(bytes.as_slice())?;
                    ws.close(None)?;
                    return Ok(res);
                }
                tungstenite::Message::Close(_) => {
                    ws.close(None)?;
                    break;
                }
                _ => {}
            }
        }
        Err(anyhow::anyhow!("No response"))
    }
}
