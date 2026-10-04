use crate::{clipx, ipc::Client};
use clap::{Args, Subcommand};

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
    Approve {
        device_id: String,
        #[arg(long, default_value_t = false, help = "Reject instead of approve")]
        deny: bool,
    },
}

impl PairArgs {
    pub fn run(args: PairArgs, ipc: &mut Client) {
        let request = match args.cmd {
            Some(PairSubcmd::Start { device_id }) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest {
                    device_id,
                })),
            },
            Some(PairSubcmd::Pending) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::PairPending(
                    clipx::PairPendingRequest {},
                )),
            },
            Some(PairSubcmd::Approve { device_id, deny }) => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::PairApprove(
                    clipx::PairApproveRequest {
                        device_id,
                        approve: !deny,
                    },
                )),
            },
            None => clipx::IpcRequest {
                request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest {
                    device_id: String::new(),
                })),
            },
        };

        if let Ok(res) = ipc.send(request) {
            match res.response {
                Some(clipx::ipc_response::Response::Pair(resp)) => {
                    println!("{}", resp.message);
                }
                Some(clipx::ipc_response::Response::PairPending(resp)) => {
                    for pending in resp.pending {
                        if pending.code.is_empty() {
                            println!(
                                "{} ({}) -> {}",
                                pending.device_id, pending.device_name, pending.stage
                            );
                        } else {
                            println!(
                                "{} ({}) -> {} [code: {}]",
                                pending.device_id, pending.device_name, pending.stage, pending.code
                            );
                        }
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
