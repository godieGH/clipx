use clap::Args;
use crate::{clipx, ipc::Client};

#[derive(Args)]
pub struct IdentityArgs {
    #[arg(long, help = "Display only the device id")]
    id: bool,

    #[arg(long, help = "Display only the device name")]
    name: bool,

    #[arg(short, long, alias = "pub", help = "Display only the device public key")]
    pub_key: bool,

    #[arg(long, alias = "ws-port", help = "Shows only the current websocket port")]
    ws_port: bool,

    #[arg(long, alias = "type", help = "Displays only the device type")]
    device_type: bool,
}

impl IdentityArgs {
    pub fn run(args: IdentityArgs, ipc: &mut Client) {
        let ipcreq = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Identity(clipx::IdentityRequest {})),
        };

        if let Ok(res) = ipc.send(ipcreq) {
            if let Some(clipx::ipc_response::Response::Identity(response)) = res.response {
                let device_type = clipx::DeviceType::try_from(response.device_type).unwrap();
                if args.id {
                    println!("{}", response.device_id);
                } else if args.name {
                    println!("{}", response.device_name);
                } else if args.pub_key {
                    println!("{}", response.public_key_hex);
                } else if args.ws_port {
                    println!("{}", response.ws_port);
                } else if args.device_type {
                    println!("{:?}", device_type);
                } else {
                    println!("id: {}", response.device_id);
                    println!("name: {}", response.device_name);
                    println!("public_key: {}", response.public_key_hex);
                    println!("ws_port: {}", response.ws_port);
                    println!("device_type: {:?}", device_type);
                    println!("ip: {}", response.ip_address);
                }
            }
        }
    }
}