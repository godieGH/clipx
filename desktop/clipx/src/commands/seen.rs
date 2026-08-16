use clap::Args;
use crate::ipc::Client;
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
    pub fn run(args: SeenArgs, ipc: &mut Client) {
        let mode = if args.trusted {
            clipx::seen_request::Mode::Trusted as i32
        } else if args.untrusted {
            clipx::seen_request::Mode::Untrusted as i32
        } else {
            clipx::seen_request::Mode::All as i32
        };

        let req = clipx::SeenRequest { mode };
        let ipcreq = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Seen(req)),
        };

        if let Ok(res) = ipc.send(ipcreq) {
            if let Some(clipx::ipc_response::Response::Seen(response)) = res.response {
                for device in response.devices {
                    if !args.verbose {
                        println!("{} ({}) - {}", device.name, device.id, device.address);
                    } else {
                        println!("Id: {}", device.id);
                        println!("  name: {}", device.name);
                        println!("  adrress: {}", device.address);
                        println!("  ws_port: {}", device.ws_port);
                        println!("  type: {:?}", device.device_type());
                        println!("  auto-connect: {:?}", device.auto_connect);
                    }
                }
            }
        }
    }
}