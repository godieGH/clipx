use crate::netio;
use crate::state::{AppState, Event};
use clipx_core::device::identity;
use clipx_core::device::pairing::PairSession;
use clipx_core::message::proto::{self, clipboard_message, peer_message::Body};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, watch};
use tokio::time::Duration;

pub fn print_event(ev: Event) {
    match ev {
        Event::Discovered {
            device_id,
            name,
            addr,
        } => println!("[disco] {device_id} '{name}' at {addr}"),
        Event::Connected { label, addr } => println!("[conn:{label}] connected ({addr})"),
        Event::Disconnected { label } => println!("[conn:{label}] disconnected"),
        Event::Message { label, message } => {
            println!("[conn:{label}] recv <- {}", describe(&message))
        }
        Event::Info(msg) => println!("[info] {msg}"),
        Event::Warn(msg) => println!("[warn] {msg}"),
    }
}

fn describe(m: &proto::PeerMessage) -> String {
    match &m.body {
        Some(Body::PairRequest(r)) => format!(
            "PairRequest{{name={}, pubkey={}}}",
            r.name,
            hex::encode(&r.public_key)
        ),
        Some(Body::PairResponse(r)) => format!(
            "PairResponse{{name={}, pubkey={}}}",
            r.name,
            hex::encode(&r.public_key)
        ),
        Some(Body::PairChallenge(c)) => format!("PairChallenge{{nonce={}}}", hex::encode(&c.nonce)),
        Some(Body::PairChallengeResponse(r)) => {
            format!("PairChallengeResponse{{sig={}}}", hex::encode(&r.signature))
        }
        Some(Body::PairAck(_)) => "PairAck{}".to_string(),
        Some(Body::ConnectChallenge(c)) => format!(
            "ConnectChallenge{{nonce={}, initiator_fp={}}}",
            hex::encode(&c.nonce),
            hex::encode(&c.initiator_fingerprint)
        ),
        Some(Body::ConnectChallengeResponse(r)) => format!(
            "ConnectChallengeResponse{{sig={}}}",
            hex::encode(&r.signature)
        ),
        Some(Body::ConnectAck(_)) => "ConnectAck{}".to_string(),
        Some(Body::Control(c)) => format!("Control{{code={}, message={}}}", c.code, c.message),
        Some(Body::Clipboard(c)) => match &c.content {
            Some(clipboard_message::Content::Text(t)) => format!("Clipboard{{text={t:?}}}"),
            Some(clipboard_message::Content::RichText(r)) => format!(
                "Clipboard{{rich_text={:?}, html_len={}}}",
                r.text,
                r.html.len()
            ),
            Some(clipboard_message::Content::Image(i)) => format!(
                "Clipboard{{image={}x{}, rgba_len={}}}",
                i.width,
                i.height,
                i.rgba.len()
            ),
            Some(clipboard_message::Content::FileOffer(f)) => format!(
                "Clipboard{{file_offer id={}, name={:?}, mime={}, size={}, expires_at_ms={}}}",
                f.file_id, f.name, f.mime_type, f.size, f.expires_at_ms
            ),
            None => "Clipboard{empty}".to_string(),
        },
        Some(Body::FileDownloadRequest(r)) => format!(
            "FileDownloadRequest{{file_id={}, offset={}}}",
            r.file_id, r.offset
        ),
        Some(Body::FileChunk(c)) => format!(
            "FileChunk{{file_id={}, seq={}, offset={}, len={}, eof={}, total_size={}}}",
            c.file_id,
            c.seq,
            c.offset,
            c.data.len(),
            c.eof,
            c.total_size
        ),
        Some(Body::FileChunkAck(a)) => format!(
            "FileChunkAck{{file_id={}, confirmed_offset={}}}",
            a.file_id, a.confirmed_offset
        ),
        Some(Body::FileTransferCancel(c)) => format!("FileTransferCancel{{file_id={}}}", c.file_id),
        None => "EMPTY".to_string(),
    }
}

pub async fn repl(state: Arc<AppState>) {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    loop {
        print!("> ");
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        let Ok(Some(line)) = lines.next_line().await else {
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "quit" | "exit") {
            break;
        }
        dispatch(&state, line).await;
    }
}

async fn dispatch(state: &Arc<AppState>, line: &str) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.as_slice() {
        ["help"] => print_help(),

        ["id"] | ["identity"] => {
            println!("name: {}", state.device_name);
            println!(
                "fingerprint: {}",
                hex::encode(state.identity.get_this_device_fingerprint())
            );
            println!(
                "public_key: {}",
                hex::encode(state.identity.public_key_bytes())
            );
        }

        ["disco", "broadcast", "on"] => start_broadcast(state, 9999, 2).await,
        ["disco", "broadcast", "on", port] => {
            start_broadcast(state, parse_or_warn(state, port, 9999), 2).await
        }
        ["disco", "broadcast", "off"] => {
            stop_task(&state.broadcast_shutdown, state, "broadcaster").await
        }

        ["disco", "listen", "on"] => start_listen(state, 9999).await,
        ["disco", "listen", "on", port] => {
            start_listen(state, parse_or_warn(state, port, 9999)).await
        }
        ["disco", "listen", "off"] => stop_task(&state.listen_shutdown, state, "listener").await,

        ["disco", "seen"] => {
            let d = state.discovered.lock().await;
            if d.is_empty() {
                println!("(nothing discovered yet)");
            }
            for (id, (a, addr)) in d.iter() {
                println!(
                    "{id} '{}' ws_port={} last_seen_from={addr}",
                    a.device_name, a.ws_port
                );
            }
        }

        ["listen-inbound", "on"] => start_inbound(state, 8081).await,
        ["listen-inbound", "on", port] => {
            start_inbound(state, parse_or_warn(state, port, 8081)).await
        }
        ["listen-inbound", "off"] => {
            stop_task(&state.inbound_shutdown, state, "inbound acceptor").await
        }

        ["dial", label, addr] => match addr.parse::<SocketAddr>() {
            Ok(addr) => {
                let state = state.clone();
                let label = label.to_string();
                tokio::spawn(async move {
                    netio::dial(state, label, addr).await;
                });
            }
            Err(e) => println!("bad address '{addr}': {e}"),
        },

        ["conns"] => {
            let conns = state.conns.lock().await;
            if conns.is_empty() {
                println!("(no open connections)");
            }
            for (label, handle) in conns.iter() {
                let sess = handle.session.lock().await;
                println!(
                    "{label} -> {} | peer_name={:?} peer_key={} nonce={}",
                    handle.addr,
                    sess.peer_name,
                    sess.peer_public_key
                        .map(hex::encode)
                        .unwrap_or_else(|| "-".into()),
                    sess.nonce.map(hex::encode).unwrap_or_else(|| "-".into()),
                );
            }
        }

        ["session", "set-peer-key", label, hexkey] => {
            let Ok(bytes) = hex::decode(hexkey) else {
                println!("bad hex");
                return;
            };
            let Ok(pk): Result<[u8; 32], _> = bytes.as_slice().try_into() else {
                println!("must be 32 bytes");
                return;
            };
            let conns = state.conns.lock().await;
            let Some(handle) = conns.get(*label) else {
                println!("no such connection: {label}");
                return;
            };
            handle.session.lock().await.peer_public_key = Some(pk);
            println!("ok");
        }

        ["send", label, "pair-request"] => send_pair_request(state, label, None).await,
        ["send", label, "pair-request", name] => {
            send_pair_request(state, label, Some(name.to_string())).await
        }

        ["send", label, "pair-response"] => {
            let own_pub = state.identity.public_key_bytes();
            let body = Body::PairResponse(proto::PeerPairResponse {
                public_key: own_pub.to_vec(),
                name: state.device_name.clone(),
            });
            report(netio::send_body(state, label, body).await);
        }

        ["send", label, "pair-challenge"] => {
            let nonce = state.identity.random_nonce();
            if let Ok(arr) = <[u8; 32]>::try_from(nonce.as_slice())
                && let Some(h) = state.conns.lock().await.get(*label)
            {
                h.session.lock().await.nonce = Some(arr);
            }
            report(
                netio::send_body(
                    state,
                    label,
                    Body::PairChallenge(proto::PeerPairChallenge { nonce }),
                )
                .await,
            );
        }

        ["send", label, "pair-challenge-response"] => {
            let Some(nonce) = session_nonce(state, label).await else {
                println!(
                    "no nonce captured on {label} yet — receive a PairChallenge first, or send one manually"
                );
                return;
            };
            let sig = state.identity.sign(&nonce);
            report(
                netio::send_body(
                    state,
                    label,
                    Body::PairChallengeResponse(proto::PeerPairChallengeResponse {
                        signature: sig.to_vec(),
                    }),
                )
                .await,
            );
        }

        ["send", label, "pair-ack"] => {
            report(netio::send_body(state, label, Body::PairAck(proto::PairAck {})).await)
        }

        ["send", label, "connect-challenge"] => {
            let nonce = state.identity.random_nonce();
            if let Ok(arr) = <[u8; 32]>::try_from(nonce.as_slice())
                && let Some(h) = state.conns.lock().await.get(*label)
            {
                h.session.lock().await.nonce = Some(arr);
            }
            let own_fp = state.identity.get_this_device_fingerprint();
            report(
                netio::send_body(
                    state,
                    label,
                    Body::ConnectChallenge(proto::PeerConnectChallenge {
                        nonce,
                        initiator_fingerprint: own_fp.to_vec(),
                    }),
                )
                .await,
            );
        }

        ["send", label, "connect-challenge-response"] => {
            let Some(nonce) = session_nonce(state, label).await else {
                println!("no nonce captured on {label} yet");
                return;
            };
            let sig = state.identity.sign(&nonce);
            report(
                netio::send_body(
                    state,
                    label,
                    Body::ConnectChallengeResponse(proto::PeerConnectChallengeResponse {
                        signature: sig.to_vec(),
                    }),
                )
                .await,
            );
        }

        ["send", label, "connect-ack"] => {
            report(netio::send_body(state, label, Body::ConnectAck(proto::ConnectAck {})).await)
        }

        ["send", label, "control", code, rest @ ..] => {
            let Ok(code) = code.parse::<u32>() else {
                println!("bad code");
                return;
            };
            let msg = rest.join(" ");
            report(
                netio::send_body(
                    state,
                    label,
                    Body::Control(proto::PreTransportControl { code, message: msg }),
                )
                .await,
            );
        }

        ["send", "*", "clip", rest @ ..] => {
            let text = rest.join(" ");
            let labels: Vec<String> = state.conns.lock().await.keys().cloned().collect();
            for label in labels {
                let body = Body::Clipboard(proto::ClipboardMessage {
                    content: Some(clipboard_message::Content::Text(text.clone())),
                });
                report(netio::send_body(state, &label, body).await);
            }
        }
        ["send", label, "clip", rest @ ..] => {
            let text = rest.join(" ");
            let body = Body::Clipboard(proto::ClipboardMessage {
                content: Some(clipboard_message::Content::Text(text)),
            });
            report(netio::send_body(state, label, body).await);
        }

        ["pair", "auto", label] => {
            let state = state.clone();
            let label = label.to_string();
            let bus = state.events_tx.clone();
            tokio::spawn(async move {
                pair_auto(&state, &bus, label).await;
            });
        }

        ["respond", "auto", "on"] => {
            state.auto_respond.store(true, Ordering::SeqCst);
            println!("auto-responder ON — inbound PairRequests will be answered end to end");
        }
        ["respond", "auto", "off"] => {
            state.auto_respond.store(false, Ordering::SeqCst);
            println!("auto-responder OFF");
        }

        _ => println!("unrecognized command — type 'help'"),
    }
}

fn parse_or_warn(state: &Arc<AppState>, s: &str, default: u16) -> u16 {
    s.parse().unwrap_or_else(|_| {
        state.emit(Event::Warn(format!("bad port '{s}', using {default}")));
        default
    })
}

fn report(res: anyhow::Result<()>) {
    if let Err(e) = res {
        println!("error: {e}");
    }
}

async fn session_nonce(state: &Arc<AppState>, label: &str) -> Option<[u8; 32]> {
    let conns = state.conns.lock().await;
    let handle = conns.get(label)?;
    handle.session.lock().await.nonce
}

async fn send_pair_request(state: &Arc<AppState>, label: &str, name: Option<String>) {
    let own_pub = state.identity.public_key_bytes();
    let own_fp = state.identity.get_this_device_fingerprint();
    let body = Body::PairRequest(proto::PeerPairRequest {
        name: name.unwrap_or_else(|| state.device_name.clone()),
        public_key: own_pub.to_vec(),
        fingerprint: own_fp.to_vec(),
    });
    report(netio::send_body(state, label, body).await);
}

async fn start_broadcast(state: &Arc<AppState>, port: u16, interval: u64) {
    let (tx, rx) = watch::channel(false);
    *state.broadcast_shutdown.lock().await = Some(tx);
    let state2 = state.clone();
    tokio::spawn(async move {
        netio::run_broadcaster(rx, state2, port, interval).await;
    });
}

async fn start_listen(state: &Arc<AppState>, port: u16) {
    let (tx, rx) = watch::channel(false);
    *state.listen_shutdown.lock().await = Some(tx);
    let state2 = state.clone();
    tokio::spawn(async move {
        netio::run_listener(rx, state2, port).await;
    });
}

async fn start_inbound(state: &Arc<AppState>, port: u16) {
    let (tx, rx) = watch::channel(false);
    *state.inbound_shutdown.lock().await = Some(tx);
    let state2 = state.clone();
    tokio::spawn(async move {
        netio::run_inbound_acceptor(rx, state2, port).await;
    });
}

async fn stop_task(
    slot: &tokio::sync::Mutex<Option<watch::Sender<bool>>>,
    state: &Arc<AppState>,
    name: &str,
) {
    if let Some(tx) = slot.lock().await.take() {
        let _ = tx.send(true);
    } else {
        state.emit(Event::Info(format!("{name} was not running")));
    }
}

// ---------------- Automated flows (mirror clipx-core's manager.rs) ----------------

async fn wait_for(
    bus: &broadcast::Sender<Event>,
    label: &str,
    timeout: Duration,
    pred: impl Fn(&proto::PeerMessage) -> bool,
) -> Option<proto::PeerMessage> {
    let mut rx = bus.subscribe();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(Event::Message { label: l, message })) if l == label && pred(&message) => {
                return Some(message);
            }
            Ok(Ok(_)) => continue,
            _ => return None,
        }
    }
}

/// Full initiator-side pairing flow, wire messages only (no UI/code-confirm
/// step exists on the wire — see manager.rs, that's a purely local gate).
async fn pair_auto(state: &Arc<AppState>, bus: &broadcast::Sender<Event>, label: String) {
    println!("[pair-auto:{label}] stage 1: sending PairRequest");
    let own_pub = state.identity.public_key_bytes();
    let own_fp = state.identity.get_this_device_fingerprint();
    if netio::send_body(
        state,
        &label,
        Body::PairRequest(proto::PeerPairRequest {
            name: state.device_name.clone(),
            public_key: own_pub.to_vec(),
            fingerprint: own_fp.to_vec(),
        }),
    )
    .await
    .is_err()
    {
        println!("[pair-auto:{label}] send failed, aborting");
        return;
    }

    let Some(resp) = wait_for(bus, &label, Duration::from_secs(30), |m| {
        matches!(m.body, Some(Body::PairResponse(_)))
    })
    .await
    else {
        println!("[pair-auto:{label}] timed out waiting for PairResponse");
        return;
    };
    let Some(Body::PairResponse(resp)) = resp.body else {
        unreachable!()
    };
    let Ok(peer_pub): Result<[u8; 32], _> = resp.public_key.as_slice().try_into() else {
        println!("[pair-auto:{label}] peer sent a malformed public key");
        return;
    };
    println!(
        "[pair-auto:{label}] got PairResponse from '{}', pubkey={}",
        resp.name,
        hex::encode(peer_pub)
    );

    println!("[pair-auto:{label}] stage 2: sending PairChallenge");
    let nonce = state.identity.random_nonce();
    let nonce_arr: [u8; 32] = nonce.as_slice().try_into().unwrap();
    if netio::send_body(
        state,
        &label,
        Body::PairChallenge(proto::PeerPairChallenge { nonce }),
    )
    .await
    .is_err()
    {
        println!("[pair-auto:{label}] send failed, aborting");
        return;
    }

    let Some(cr) = wait_for(bus, &label, Duration::from_secs(60), |m| {
        matches!(m.body, Some(Body::PairChallengeResponse(_)))
    })
    .await
    else {
        println!(
            "[pair-auto:{label}] timed out waiting for PairChallengeResponse (peer may be waiting on its own local user's approval popup)"
        );
        return;
    };
    let Some(Body::PairChallengeResponse(cr)) = cr.body else {
        unreachable!()
    };

    if !identity::verify(&peer_pub, &nonce_arr, &cr.signature) {
        println!("[pair-auto:{label}] signature INVALID — NOT sending Ack, aborting");
        return;
    }
    let code = PairSession::compute_code(&nonce_arr, &own_pub, &peer_pub);
    println!(
        "[pair-auto:{label}] signature verified. (the code a real UI would show here: {code:06})"
    );
    let _ = netio::send_body(state, &label, Body::PairAck(proto::PairAck {})).await;
    println!("[pair-auto:{label}] sent PairAck — pairing flow complete from this side");
}

/// Runs continuously in the background once 'respond auto on' is set —
/// answers any inbound PairRequest end to end, mirroring the responder side
/// of manager.rs (on_pair_request / on_pair_challenge / on_pair_ack).
pub async fn auto_responder_loop(state: Arc<AppState>) {
    let mut rx = state.events_tx.subscribe();
    loop {
        let Ok(ev) = rx.recv().await else { break };
        if !state.auto_respond.load(Ordering::SeqCst) {
            continue;
        }
        if let Event::Message { label, message } = ev
            && let Some(Body::PairRequest(req)) = message.body
        {
            let state = state.clone();
            let bus = state.events_tx.clone();
            tokio::spawn(async move {
                respond_to_pair(state, bus, label, req).await;
            });
        }
    }
}

async fn respond_to_pair(
    state: Arc<AppState>,
    bus: broadcast::Sender<Event>,
    label: String,
    req: proto::PeerPairRequest,
) {
    println!("[auto-respond:{label}] got PairRequest from '{}'", req.name);
    let Ok(peer_pub): Result<[u8; 32], _> = req.public_key.as_slice().try_into() else {
        println!("[auto-respond:{label}] malformed public key, ignoring");
        return;
    };
    let own_pub = state.identity.public_key_bytes();
    if netio::send_body(
        &state,
        &label,
        Body::PairResponse(proto::PeerPairResponse {
            public_key: own_pub.to_vec(),
            name: state.device_name.clone(),
        }),
    )
    .await
    .is_err()
    {
        return;
    }
    println!("[auto-respond:{label}] sent PairResponse");

    let Some(ch) = wait_for(&bus, &label, Duration::from_secs(30), |m| {
        matches!(m.body, Some(Body::PairChallenge(_)))
    })
    .await
    else {
        println!("[auto-respond:{label}] timed out waiting for PairChallenge");
        return;
    };
    let Some(Body::PairChallenge(c)) = ch.body else {
        unreachable!()
    };
    let Ok(nonce_arr): Result<[u8; 32], _> = c.nonce.as_slice().try_into() else {
        return;
    };
    let sig = state.identity.sign(&nonce_arr);
    let code = PairSession::compute_code(&nonce_arr, &own_pub, &peer_pub);
    println!(
        "[auto-respond:{label}] signing challenge (the code a real UI would show here: {code:06})"
    );
    if netio::send_body(
        &state,
        &label,
        Body::PairChallengeResponse(proto::PeerPairChallengeResponse {
            signature: sig.to_vec(),
        }),
    )
    .await
    .is_err()
    {
        return;
    }

    match wait_for(&bus, &label, Duration::from_secs(30), |m| {
        matches!(m.body, Some(Body::PairAck(_)))
    })
    .await
    {
        Some(_) => println!("[auto-respond:{label}] received PairAck — pairing complete"),
        None => println!("[auto-respond:{label}] no Ack received (timed out)"),
    }
}

fn print_help() {
    println!(
        r#"
identity/device
  id                                  show this emulator's identity

discovery (UDP)
  disco broadcast on [port=9999]      start broadcasting Announce every 2s
  disco broadcast off
  disco listen on [port=9999]         start listening for Announces
  disco listen off
  disco seen                          list devices discovered so far

transport (TCP+WS)
  listen-inbound on [port=8081]       accept inbound peer connections (act as responder), labels: in-N
  listen-inbound off
  dial <label> <ip:port>              open an outbound connection, store as <label>
  conns                               list open connections + captured session info
  session set-peer-key <label> <hex>  manually inject a known peer pubkey (for connect-flow testing)

raw staged sends — test any single stage in isolation, in any order
  send <label> pair-request [name]
  send <label> pair-response
  send <label> pair-challenge                    (stage 2 / "challenge stage")
  send <label> pair-challenge-response
  send <label> pair-ack
  send <label> connect-challenge
  send <label> connect-challenge-response
  send <label> connect-ack
  send <label> control <code> <message>

clipboard
  send <label> clip <text...>                    send a Clipboard{{text}} frame — no pairing/history required

automated end-to-end flows
  pair auto <label>                   run the full initiator pairing flow against <label>
  respond auto on|off                 auto-answer any inbound PairRequest end to end

quit / exit
"#
    );
}
