use clap::Args;
use crate::ipc::IpcClient;
use crate::clipx;

#[derive(Args, Debug)]
pub struct SeenArgs {

    #[arg(short, long, help = "Display all the untrusted currently discovered devices")]
    untrusted: bool,

    #[arg(short, long, help = "Display all the trusted currently discovered devices")]
    trusted: bool,

    #[arg(long, help = "Display with extra verbose information")]
    verbose: bool,
}

impl SeenArgs {
    pub fn run(args: SeenArgs, mut ipc: IpcClient) {
        let mode = {
            if args.trusted {
                clipx::seen_request::Mode::Trusted.into()
            }
            else if args.untrusted {
                clipx::seen_request::Mode::Untrusted.into()
            }
            else {
                clipx::seen_request::Mode::All.into()
            }
        };
        let req = clipx::SeenRequest {mode};
        let ipcreq = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Seen(req))
        };

        let _res = ipc.send(ipcreq).unwrap();

 
        
    }
}