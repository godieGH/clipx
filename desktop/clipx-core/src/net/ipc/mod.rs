use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, ToNsName,
    tokio::{Listener, Stream as LocalStream},
    traits::tokio::Listener as ListenerTrait,
};
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch};
use tokio_tungstenite::{accept_async, WebSocketStream, tungstenite::Message as WsMessage};
use futures_util::{SinkExt, StreamExt};

use crate::{device::manager::DeviceCommands, message::proto::clipx};
use prost::Message;

// starts and manage ipc resources
pub struct IpcService {
    name: String,
    listener: Option<Listener>,
    core_service_shutdown_rx: watch::Receiver<bool>,
    device_tx: UnboundedSender<DeviceCommands>
}

impl IpcService {
    pub fn new(
        name: impl Into<String>,
        shutdown_rx: watch::Receiver<bool>,
        device_tx: UnboundedSender<DeviceCommands>
    ) -> Self {
        Self {
            name: name.into(),
            listener: None,
            core_service_shutdown_rx: shutdown_rx,
            device_tx,
        }
    }

    pub async fn start(&mut self) {
        let name = self.create_name().unwrap();
        let listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        self.listener = Some(listener);

        tracing::info!("IPC services started");

        loop {
            tokio::select! {
                _ = self.core_service_shutdown_rx.changed() => {
                    if *self.core_service_shutdown_rx.borrow() {
                        break;
                    }
                }
                conn = self.listener.as_ref().unwrap().accept() => {
                    match conn {
                        Ok(stream) => {
                            if let Err(err) = self.handle_client(stream).await {
                                tracing::error!(error = %err, "IPC client handler failed");
                            }
                        }
                        Err(e) => tracing::error!("Accept error: {e:?}"),
                    }
                }
            }
        }
    }

    async fn handle_client(&self, conn: LocalStream) -> anyhow::Result<()> {
        let mut ws = accept_async(conn).await?;

        while let Some(msg) = ws.next().await {
            let msg = msg?;
            if  msg.is_close() {break;}

            match msg {
                WsMessage::Binary(bytes) => {
                    tracing::info!("Ipc request made");
                    let ipcreq = IpcService::decode_ipc_req(bytes.to_vec())?;
                    self.handle_ipc_request(ipcreq, &mut ws).await?;
                }
                _ => {
                    break;
                }

            }
        }

        ws.close(None).await?;
        Ok(())
    }

    async fn handle_ipc_request(
        &self,
        ipcreq: clipx::IpcRequest,
        ws: &mut WebSocketStream<LocalStream>
    ) -> anyhow::Result<()> {
        let (cmdres_tx, cmdres_rx) = oneshot::channel();
        let req = match ipcreq.request {
            Some(req) => req,
            None => return Err(anyhow::anyhow!("Invalid Ipc request"))
        };

        match req {
            clipx::ipc_request::Request::Seen(clipx::SeenRequest {mode}) => {
                use clipx::seen_request::Mode;
                let mode = Mode::try_from(mode)?;
                match mode {
                    Mode::All => {
                        //let buf = Vec::new();
                        let _ = self
                            .device_tx
                            .send(DeviceCommands::GetSeen { reply_to: cmdres_tx });
                        


                        //ws.send(WsMessage::Binary(buf)).await?;
                        ws.close(None).await?;
                    }
                    Mode::Trusted => {}
                    Mode::Untrusted => {}
                }

            }
            _ => {
                return Err(anyhow::anyhow!("Invalid ipc req"));
            }
        }



        Ok(())
    }
}


// other helpers
impl IpcService {
    fn create_name(&self) -> std::io::Result<Name<'_>> {
        self.name.clone().to_ns_name::<GenericNamespaced>()
    }

    fn decode_ipc_req(bytes: Vec<u8>) -> anyhow::Result<clipx::IpcRequest> {
        Ok(clipx::IpcRequest::decode(bytes.as_slice())?)
    }
}