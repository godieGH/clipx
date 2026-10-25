use crate::{clipx, ipc::Client};
use clap::Args;

#[derive(Args)]
pub struct ConnectArgs {
    #[arg(long, help = "List currently connected devices")]
    list: bool,

    #[arg(long, help = "Disconnect from a connected device")]
    disconnect: bool,

    #[arg(
        value_name = "DEVICE_ID",
        help = "Connect to or disconnect from a trusted device"
    )]
    device_id: Option<String>,
}

impl ConnectArgs {
    pub fn run(args: ConnectArgs, ipc: &mut Client) {
        let request = if args.list {
            clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Connected(
                    clipx::ConnectedRequest {},
                )),
            }
        } else if let Some(device_id) = args.device_id {
            if args.disconnect {
                clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::Disconnect(
                        clipx::DisconnectRequest { device_id },
                    )),
                }
            } else {
                clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::Connect(
                        clipx::ConnectRequest { device_id },
                    )),
                }
            }
        } else {
            clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Connected(
                    clipx::ConnectedRequest {},
                )),
            }
        };

        if let Ok(res) = ipc.send(request) {
            match res.response {
                Some(clipx::ipc_response::Response::Connect(resp)) => {
                    println!("{}", resp.message);
                }
                Some(clipx::ipc_response::Response::Disconnect(resp)) => {
                    println!("{}", resp.message);
                }
                Some(clipx::ipc_response::Response::Connected(resp)) => {
                    if resp.devices.is_empty() {
                        println!("No connected devices.");
                        return;
                    }
                    for device in resp.devices {
                        println!("{} ({})", device.name, device.id);
                    }
                }
                _ => {}
            }
        }
    }
}
