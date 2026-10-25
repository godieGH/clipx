use crate::{clipx, ipc::Client};
use clap::Args;

#[derive(Args, Debug)]
pub struct DownloadArgs {
    #[arg(
        value_name = "ENTRY_ID",
        help = "Clipboard history entry id to download"
    )]
    entry_id: String,
}

impl DownloadArgs {
    pub fn run(args: DownloadArgs, ipc: &mut Client) {
        let request = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::DownloadClipboardFile(
                clipx::DownloadClipboardFileRequest {
                    entry_id: args.entry_id,
                },
            )),
        };

        if let Ok(res) = ipc.send(request) {
            if let Some(clipx::ipc_response::Response::DownloadClipboardFile(response)) =
                res.response
            {
                println!("{}", response.message);
            }
        }
    }
}
