mod commands;
mod ipc;
mod clipx {
    #![allow(unused)]
    mod clipx {
        include!(concat!(env!("OUT_DIR"), "/clipx.rs"));
    }
    
    pub use clipx::*;
    
}

use clap::{Parser, Subcommand};
use commands::{seen::SeenArgs, trusted::TrustedArgs, identity::IdentityArgs, pair::PairArgs};

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
}

fn main() {
    let cli = Cli::parse();
    let ipc = ipc::IpcClient::new("clipx").run();

    match cli.cmd {
        Cmd::Seen(args) => {
            SeenArgs::run(args, ipc);
        }
        Cmd::Trusted(args) => {
            commands::trusted::TrustedArgs::run(args, ipc);
        }
        Cmd::Identity(args) => {
            commands::identity::IdentityArgs::run(args, ipc);
        }
        Cmd::Pair(_args) => {}
    }
}

mod utils {
    #![allow(unused)]
    use serde::{Deserialize, Serialize, de::DeserializeOwned};

    pub fn serialize(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }

    pub fn deserialize<T>(value: &str) -> anyhow::Result<T> 
    where T: DeserializeOwned {
        Ok(serde_json::from_str(value)?)
    }
}