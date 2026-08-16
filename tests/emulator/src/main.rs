mod commands;
mod netio;
mod state;

use clipx_core::device::identity::DeviceIdentity;
use state::AppState;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

#[tokio::main]
async fn main() {
    let mut name = "emulator".to_string();
    let mut id_path = std::env::temp_dir().join("clipx_emulator_identity_default");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => { if let Some(v) = args.next() { name = v; } }
            "--id-path" => { if let Some(v) = args.next() { id_path = v.into(); } }
            "-h" | "--help" => {
                println!("usage: clipx-protocol-emulator [--name NAME] [--id-path PATH]");
                println!("  --name NAME     display name this emulator announces/pairs as (default: emulator)");
                println!("  --id-path PATH  where to persist this emulator's keypair, so its identity");
                println!("                  is stable across restarts (default: a per-name temp file)");
                return;
            }
            other => { eprintln!("unrecognized flag: {other} (try --help)"); return; }
        }
    }
    // Default id path is per-name so you can run several emulator instances
    // side by side (e.g. one as initiator, one as responder) without them
    // colliding on the same identity file.
    if id_path == std::env::temp_dir().join("clipx_emulator_identity_default") {
        id_path = std::env::temp_dir().join(format!("clipx_emulator_identity_{name}"));
    }

    let identity = Arc::new(DeviceIdentity::load_or_create(id_path.clone()));
    let (events_tx, mut printer_rx) = broadcast::channel(1024);

    let state = Arc::new(AppState {
        identity: identity.clone(),
        device_name: name.clone(),
        discovered: Mutex::new(Default::default()),
        conns: Mutex::new(Default::default()),
        events_tx,
        next_inbound: AtomicU32::new(0),
        auto_respond: AtomicBool::new(false),
        broadcast_shutdown: Mutex::new(None),
        listen_shutdown: Mutex::new(None),
        inbound_shutdown: Mutex::new(None),
    });

    println!("clipx protocol emulator — device '{name}'");
    println!("identity file: {id_path:?}");
    println!("fingerprint:   {}", hex::encode(identity.get_this_device_fingerprint()));
    println!("type 'help' for commands\n");

    tokio::spawn(async move {
        use std::io::Write as _;
        loop {
            match printer_rx.recv().await {
                Ok(ev) => {
                    // Background events (a discovery, an inbound connection, a
                    // received message) can land between keystrokes at any
                    // time — reprint the prompt after each one so it doesn't
                    // look like the REPL swallowed your input.
                    println!();
                    commands::print_event(ev);
                    print!("> ");
                    let _ = std::io::stdout().flush();
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tokio::spawn(commands::auto_responder_loop(state.clone()));

    commands::repl(state).await;
}
