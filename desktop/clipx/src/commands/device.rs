use crate::{clipx, ipc::Client};
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct DeviceArgs {
    #[command(subcommand)]
    cmd: DeviceSubcmd,
}

#[derive(Subcommand, Debug)]
enum DeviceSubcmd {
    #[command(about = "List paired devices")]
    List,
    #[command(about = "List discovered devices")]
    Seen {
        #[arg(long, help = "Only show trusted devices")]
        trusted: bool,
        #[arg(long, help = "Display extra device metadata")]
        verbose: bool,
    },
    #[command(about = "Forget a paired device")]
    Forget { device_id: String },
    #[command(about = "Set whether a device auto-connects")]
    AutoConnect {
        device_id: String,
        #[arg(long, default_value_t = true)]
        enable: bool,
    },
}

impl DeviceArgs {
    pub fn run(args: DeviceArgs, ipc: &mut Client) {
        match args.cmd {
            DeviceSubcmd::List => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::GetPaired(
                        clipx::GetPairedRequest {},
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::GetPaired(response)) = res.response {
                        if response.devices.is_empty() {
                            println!("No paired devices found.");
                            return;
                        }

                        for device in response.devices {
                            let state = clipx::ConnectionState::try_from(device.connection)
                                .unwrap_or(clipx::ConnectionState::Disconnected);
                            println!(
                                "{} ({}) - {} [{:?}]",
                                device.name, device.id, device.address, state
                            );
                        }
                    }
                }
            }
            DeviceSubcmd::Seen { trusted, verbose } => {
                let mode = if trusted {
                    clipx::seen_request::Mode::Trusted as i32
                } else {
                    clipx::seen_request::Mode::Untrusted as i32
                };

                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::Seen(clipx::SeenRequest {
                        mode,
                    })),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::Seen(response)) = res.response {
                        if response.devices.is_empty() {
                            println!("No discovered devices found.");
                            return;
                        }

                        for device in response.devices {
                            if verbose {
                                println!("id: {}", device.id);
                                println!("  name: {}", device.name);
                                println!("  address: {}", device.address);
                                println!("  ws_port: {}", device.ws_port);
                                println!(
                                    "  type: {:?}",
                                    clipx::DeviceType::try_from(device.device_type)
                                        .unwrap_or(clipx::DeviceType::Unspecified)
                                );
                                println!("  auto-connect: {}", device.auto_connect);
                            } else {
                                println!("{} ({}) - {}", device.name, device.id, device.address);
                            }
                        }
                    }
                }
            }
            DeviceSubcmd::Forget { device_id } => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::ForgetDevice(
                        clipx::ForgetDeviceRequest { device_id },
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::ForgetDevice(response)) =
                        res.response
                    {
                        println!("{}: {}", response.device_id, response.status);
                    }
                }
            }
            DeviceSubcmd::AutoConnect { device_id, enable } => {
                let request = clipx::IpcRequest {
                    request: Some(clipx::ipc_request::Request::SetAutoConnect(
                        clipx::SetAutoConnectRequest {
                            device_id,
                            auto_connect: enable,
                        },
                    )),
                };

                if let Ok(res) = ipc.send(request) {
                    if let Some(clipx::ipc_response::Response::SetAutoConnect(response)) =
                        res.response
                    {
                        println!(
                            "{} auto-connect={}",
                            response.device_id, response.auto_connect
                        );
                    }
                }
            }
        }
    }
}
