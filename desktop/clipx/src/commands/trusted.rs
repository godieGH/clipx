use crate::{clipx, ipc::Client};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct TrustedArgs {
    #[command(subcommand)]
    cmd: Option<TrustedSubcmd>,
}

#[derive(Subcommand)]
enum TrustedSubcmd {
    #[command(
        alias = "unpair",
        about = "Disconnect, unpair and remove a device from trusted"
    )]
    Remove { id: String },
}

impl TrustedArgs {
    pub fn run(args: TrustedArgs, ipc: &mut Client) {
        let request = match args.cmd {
            Some(TrustedSubcmd::Remove { id }) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::ForgetDevice(
                    clipx::ForgetDeviceRequest { device_id: id },
                )),
            },
            None => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Trusted(
                    clipx::TrustedRequest {},
                )),
            },
        };

        if let Ok(res) = ipc.send(request) {
            match res.response {
                Some(clipx::ipc_response::Response::Trusted(response)) => {
                    for device in response.devices {
                        println!("{} ({})", device.name, device.id);
                    }
                }
                Some(clipx::ipc_response::Response::ForgetDevice(response)) => {
                    println!("{}: {}", response.device_id, response.status);
                }
                _ => {}
            }
        }
    }
}
