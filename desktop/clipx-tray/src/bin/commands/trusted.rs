use clap::{Args, Subcommand};
use crate::{clipx, ipc::Client};

#[derive(Args)]
pub struct TrustedArgs {
    #[command(subcommand)]
    cmd: Option<TrustedSubcmd>,
}

#[derive(Subcommand)]
enum TrustedSubcmd {
    #[command(alias = "unpair", about = "Disconnect, unpair and remove a device from trusted")]
    Remove {
        id: String
    }
}

impl TrustedArgs {
    pub fn run(args: TrustedArgs, ipc: &mut Client) {
        let ipcreq = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Trusted(clipx::TrustedRequest {})),
        };

        if let Ok(res) = ipc.send(ipcreq) {
            if let Some(clipx::ipc_response::Response::Trusted(response)) = res.response {
                for device in response.devices {
                    println!("{} ({})", device.name, device.id);
                }
            }
        }

        let _ = args.cmd;
    }
}