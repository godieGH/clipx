mod commands;
use clap::{Parser, Subcommand};
use clipx_lib::clipx;
use clipx_lib::ipc;
use commands::{
    clipboard::ClipboardArgs, connect::ConnectArgs, device::DeviceArgs,
    download::DownloadArgs, identity::IdentityArgs, pair::PairArgs, seen::SeenArgs,
    send::SendArgs, trusted::TrustedArgs,
};

/**
 * Command-line entry point for the desktop client.
 *
 * This tool exposes a small surface for querying peer state, pairing devices,
 * and issuing connection commands through the shared Clipx IPC layer.
 */
#[derive(Parser)]
#[command(
    about = "clipx is a cli tool to manage devices or work with clipx-core through a command line tool"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Supported actions available through the desktop CLI.
#[derive(Subcommand)]
enum Cmd {
    #[command(about = "Displays all currently discovered devices")]
    Seen(SeenArgs),
    #[command(about = "Display all trusted devices (paired devices)")]
    Trusted(TrustedArgs),
    #[command(about = "Displays this device identity")]
    Identity(IdentityArgs),
    #[command(about = "Pair with another device")]
    Pair(PairArgs),
    #[command(about = "Connect to or disconnect from a trusted device")]
    Connect(ConnectArgs),
    #[command(about = "List or manage paired devices")]
    Device(DeviceArgs),
    #[command(about = "Manage clipboard history and downloaded files")]
    Clipboard(ClipboardArgs),
    #[command(about = "Send content to another connected device")]
    Send(SendArgs),
    #[command(about = "Download a clipboard file by history entry id")]
    Download(DownloadArgs),
}

/// Starts the desktop CLI and dispatches the selected action to the IPC client.
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
        Cmd::Device(args) => {
            commands::device::DeviceArgs::run(args, &mut ipc);
        }
        Cmd::Clipboard(args) => {
            commands::clipboard::ClipboardArgs::run(args, &mut ipc);
        }
        Cmd::Send(args) => {
            commands::send::SendArgs::run(args, &mut ipc);
        }
        Cmd::Download(args) => {
            commands::download::DownloadArgs::run(args, &mut ipc);
        }
    }
}
