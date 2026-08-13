mod commands;
use clipx_tray_lib::ipc;
use clipx_tray_lib::clipx;
use clap::{Parser, Subcommand};
use commands::{seen::SeenArgs, trusted::TrustedArgs, identity::IdentityArgs, pair::PairArgs, connect::ConnectArgs};

#[derive(Parser)]
#[command(about = "clipx is a cli tool to manage devices or work with clipx-core through a command line tool")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
   #[command(about = "Displays all currently discovered devices")]
   Seen(SeenArgs),
   #[command(about = "Display all trusted devices(paired devices)")]
   Trusted(TrustedArgs),
   #[command(about = "Displays this device Identity")]
   Identity(IdentityArgs),
   #[command(about = "Pair with another devices")]
   Pair(PairArgs),
   #[command(about = "Connect to a trusted device")]
   Connect(ConnectArgs),
}

fn main() {
    let cli = Cli::parse();
    let ipc = ipc::Client::new("clipx").run();

    match cli.cmd {
        Cmd::Seen(args) => {
            commands::seen::SeenArgs::run(args, ipc);
        }
        Cmd::Trusted(args) => {
            commands::trusted::TrustedArgs::run(args, ipc);
        }
        Cmd::Identity(args) => {
            commands::identity::IdentityArgs::run(args, ipc);
        }
        Cmd::Pair(args) => {
            commands::pair::PairArgs::run(args, ipc);
        }
        Cmd::Connect(args) => {
            commands::connect::ConnectArgs::run(args, ipc);
        }
    }
}