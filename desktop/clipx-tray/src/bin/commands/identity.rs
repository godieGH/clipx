use clap::Args;

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