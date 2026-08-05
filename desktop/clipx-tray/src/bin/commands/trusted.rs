use clap::{Args, Subcommand};

#[derive(Args)]
pub struct TrustedArgs {
    #[command(subcommand)]
    cmd: Option<TrustedSubcmd>,
}

#[derive(Subcommand)]
enum TrustedSubcmd {
    #[command(alias = "unpair", about = "Disconnect, unpair and remove a device from trusted")]
    Remove {
        id: String
    }
}