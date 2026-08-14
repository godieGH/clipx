use clap::{Args, Subcommand};
use crate::{clipx, ipc::Client};

#[derive(Args)]
pub struct PairArgs {
    #[command(subcommand)]
    cmd: Option<PairSubcmd>,
}

#[derive(Subcommand)]
enum PairSubcmd {
    #[command(about = "Start a pairing request for a discovered device")]
    Start { device_id: String },
    #[command(about = "List pending pairing requests")]
    Pending,
    #[command(about = "Approve a pending pairing request")]
    Approve { device_id: String },
}

impl PairArgs {
    pub fn run(args: PairArgs, ipc: &mut Client) {
        let request = match args.cmd {
            Some(PairSubcmd::Start { device_id }) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest { device_id })),
            },
            Some(PairSubcmd::Pending) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::PairPending(clipx::PairPendingRequest {})),
            },
            Some(PairSubcmd::Approve { device_id }) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::PairApprove(clipx::PairApproveRequest { device_id })),
            },
            None => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest { device_id: String::new() })),
            },
        };

        if let Ok(res) = ipc.send(request) {
            match res.response {
                Some(clipx::ipc_response::Response::Pair(resp)) => {
                    println!("{}", resp.message);
                }
                Some(clipx::ipc_response::Response::PairPending(resp)) => {
                    for pending in resp.pending {
                        println!("{} -> {}", pending.device_id, pending.status);
                    }
                }
                Some(clipx::ipc_response::Response::PairApprove(resp)) => {
                    println!("{}", resp.message);
                }
                _ => {}
            }
        }
    }
}