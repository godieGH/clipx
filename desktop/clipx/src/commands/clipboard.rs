use crate::{clipx, ipc::Client};
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct ClipboardArgs {
    #[command(subcommand)]
    cmd: ClipboardSubcmd,
}

#[derive(Subcommand, Debug)]
enum ClipboardSubcmd {
    #[command(about = "List clipboard history entries")]
    History {
        #[arg(long, default_value_t = 0, help = "Maximum number of entries to fetch")]
        limit: u32,
    },
    #[command(about = "Remove a specific clipboard history item")]
    Remove { id: String },
    #[command(about = "Clear the clipboard history")]
    Clear,
}

impl ClipboardArgs {
    pub fn run(args: ClipboardArgs, ipc: &mut Client) {
        match args.cmd {
            ClipboardSubcmd::History { limit } => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::ClipboardHistory(
                        clipx::ClipboardHistoryRequest { limit },
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::ClipboardHistory(response)) =
                        res.response
                    {
                        if response.entries.is_empty() {
                            println!("Clipboard history is empty.");
                            return;
                        }

                        for entry in response.entries {
                            println!(
                                "{} [{}] from {}",
                                entry.id, entry.kind, entry.source_device_name
                            );
                            if !entry.content.is_empty() {
                                println!("  {}", entry.content);
                            }
                            if !entry.file_name.is_empty() {
                                println!("  file: {} ({})", entry.file_name, entry.mime_type);
                            }
                        }
                    }
                }
            }
            ClipboardSubcmd::Remove { id } => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::ClipboardRemove(
                        clipx::ClipboardRemoveRequest { id },
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::ClipboardRemove(response)) =
                        res.response
                    {
                        println!("removed={}", response.removed);
                    }
                }
            }
            ClipboardSubcmd::Clear => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::ClipboardClear(
                        clipx::ClipboardClearRequest {},
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::ClipboardClear(_)) = res.response {
                        println!("Clipboard history cleared.");
                    }
                }
            }
        }
    }
}
