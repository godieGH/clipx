mod commands;
use clap::{Parser, Subcommand};
use clipx_lib::clipx;
use clipx_lib::ipc;
use commands::{
    connect::ConnectArgs, identity::IdentityArgs, pair::PairArgs, seen::SeenArgs,
    trusted::TrustedArgs,
};

#[derive(Parser)]
#[command(
    about = "clipx is a cli tool to manage devices or work with clipx-core through a command line tool"
)]
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
    let mut ipc = ipc::Client::new("clipx").start();

    match cli.cmd {
        Cmd::Seen(args) => {
            commands::seen::SeenArgs::run(args, &mut ipc);
        }
        Cmd::Trusted(args) => {
            commands::trusted::TrustedArgs::run(args, &mut ipc);
        }
        Cmd::Identity(args) => {
            commands::identity::IdentityArgs::run(args, &mut ipc);
        }
        Cmd::Pair(args) => {
            commands::pair::PairArgs::run(args, &mut ipc);
        }
        Cmd::Connect(args) => {
            commands::connect::ConnectArgs::run(args, &mut ipc);
        }
    }
}
