pub mod non_blocking;

use interprocess::local_socket::{GenericNamespaced, Name, Stream, prelude::*, ToNsName};
use tungstenite::{client, WebSocket};
use crate::clipx;
use prost::Message;

pub struct Client {
    name: String,
    ws: Option<WebSocket<Stream>>,
    state: State,
}

enum State {
    Running,
    Down,
}

/// Please run the core for this tests examples in this module docs to work
impl Client {
    /// This creates a new empty ipc::Client
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ws: None,
            state: State::Down
        }
    }

    /// This consumes the ipc client instance and returns it back as return value
    /// ```
    /// use clipx_tray_lib::ipc::Client;
    /// // Usually call new + run on the same line to get a running client before persisting or
    /// // do something else with the instance 
    /// // The ipc instance in non-running state is usually domant
    /// let ipc = Client::new("clipx").run(); 
    /// ```
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

    /// This sends message over the ipc channel
    /// Usually only accept protobuf Message (One that `impl prost::Messag`e trait that can be encoded or decoded back to raw bytes and rust structs)
    /// Serializable Messages should be defined in protos/* directory in the root of this project
    /// This function returns `Result<Response, anyhow::Error>`. So we have to wait for the result to come blockingly
    /// Unfortunately this is blocking, waiting for the response to come through an IPC channel this will freeze the thread hence better use non-blocking version
    /// If need send that yields and await in an async runtime like tokio
    /// ```
    /// use clipx_tray_lib::{clipx, ipc::Client};
    /// 
    /// let req = clipx::IpcRequest {
    ///    request: Some(clipx::ipc_request::Request::Identity(clipx::IdentityRequest {})),
    /// };
    /// 
    /// let mut ipc = Client::new("clipx").run(); // ensure there is clipx.sock server for this to work
    /// 
    /// let res = ipc.send(req).unwrap();
    /// 
    /// ```
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
                    loop {
                        let msg = ws.read();
                        match msg {                       
                            Ok(tungstenite::Message::Close(_)) => {
                                break;
                            }
                            Ok(_) => continue,
                            Err(_) => break,
                        }

                    }
                    return Ok(res);
                }
                tungstenite::Message::Close(_) => {
                    break;
                }
                _ => {}
            }
        }
        Err(anyhow::anyhow!("No response"))
    }
}
