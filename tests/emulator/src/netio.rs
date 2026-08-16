use crate::state::{AppState, ConnHandle, Event, SessionInfo};
use clipx_core::message::proto::{self, peer_message::Body};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use socket2::{Domain, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::Duration;
use tokio_tungstenite::{accept_async, client_async, tungstenite::Message as WsMessage, WebSocketStream};

/// Same shape as clipx-core's own `make_shared_socket`, but the port is a
/// parameter, and SO_REUSEADDR (+ SO_REUSEPORT on unix) is set so this can
/// bind port 9999 alongside a real clipx-core instance's own listener.
fn make_udp_socket(port: u16) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    let _ = socket.set_reuse_port(true);
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    let addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into();
    socket.bind(&addr.into())?;
    UdpSocket::from_std(socket.into())
}

// ---------------- Discovery: broadcast component ----------------

pub async fn run_broadcaster(mut shutdown_rx: watch::Receiver<bool>, state: Arc<AppState>, port: u16, interval_secs: u64) {
    let socket = match make_udp_socket(0) {
        Ok(s) => s,
        Err(e) => { state.emit(Event::Warn(format!("broadcast socket failed: {e}"))); return; }
    };
    let announce = proto::Announce {
        fingerprint: state.identity.get_this_device_fingerprint().to_vec(),
        device_name: state.device_name.clone(),
        device_type: proto::DeviceType::Unspecified as i32,
        ws_port: 0,
    };
    let mut buf = Vec::new();
    announce.encode(&mut buf).unwrap();
    let target = format!("255.255.255.255:{port}");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
            _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                let _ = socket.send_to(&buf, &target).await;
                state.emit(Event::Info(format!("broadcast: sent Announce ({} bytes) to {target}", buf.len())));
            }
        }
    }
    state.emit(Event::Info("broadcaster stopped".into()));
}

// ---------------- Discovery: listen component ----------------

pub async fn run_listener(mut shutdown_rx: watch::Receiver<bool>, state: Arc<AppState>, port: u16) {
    let socket = match make_udp_socket(port) {
        Ok(s) => s,
        Err(e) => { state.emit(Event::Warn(format!("listen bind({port}) failed: {e}"))); return; }
    };
    state.emit(Event::Info(format!("listening for Announces on {port}")));
    let mut buf = [0u8; 1024];
    let self_fp = state.identity.get_this_device_fingerprint();

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
            result = socket.recv_from(&mut buf) => {
                let Ok((len, src)) = result else { continue };
                let announce = match proto::Announce::decode(&buf[..len]) {
                    Ok(a) => a,
                    Err(e) => { state.emit(Event::Warn(format!("undecodable announce from {src}: {e}"))); continue; }
                };
                if announce.fingerprint == self_fp { continue; }
                let device_id = hex::encode(&announce.fingerprint);
                state.discovered.lock().await.insert(device_id.clone(), (announce.clone(), src));
                state.emit(Event::Discovered { device_id, name: announce.device_name, addr: src });
            }
        }
    }
    state.emit(Event::Info("listener stopped".into()));
}

// ---------------- Transport: dial (outbound) ----------------

pub async fn dial(state: Arc<AppState>, label: String, addr: SocketAddr) {
    let stream = match TcpStream::connect(addr).await {
        Ok(s) => s,
        Err(e) => { state.emit(Event::Warn(format!("connect to {addr} failed: {e}"))); return; }
    };
    let ws = match client_async(format!("ws://{addr}/"), stream).await {
        Ok((ws, _)) => ws,
        Err(e) => { state.emit(Event::Warn(format!("ws handshake with {addr} failed: {e}"))); return; }
    };
    register_and_run(state, label, addr, ws).await;
}

// ---------------- Transport: inbound acceptor (act as responder) ----------------

pub async fn run_inbound_acceptor(mut shutdown_rx: watch::Receiver<bool>, state: Arc<AppState>, port: u16) {
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => { state.emit(Event::Warn(format!("inbound bind({port}) failed: {e}"))); return; }
    };
    state.emit(Event::Info(format!("accepting inbound peer connections on {port} (labels: in-N)")));

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => { if *shutdown_rx.borrow() { break; } }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => {
                        let state = state.clone();
                        tokio::spawn(async move { handle_inbound(state, stream, addr).await; });
                    }
                    Err(e) => state.emit(Event::Warn(format!("accept error: {e}"))),
                }
            }
        }
    }
    state.emit(Event::Info("inbound acceptor stopped".into()));
}

async fn handle_inbound(state: Arc<AppState>, stream: TcpStream, addr: SocketAddr) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => { state.emit(Event::Warn(format!("inbound ws handshake failed from {addr}: {e}"))); return; }
    };
    let id = state.next_inbound.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    register_and_run(state, format!("in-{id}"), addr, ws).await;
}

async fn register_and_run(state: Arc<AppState>, label: String, addr: SocketAddr, ws: WebSocketStream<TcpStream>) {
    let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
    let handle = Arc::new(ConnHandle { addr, outbound_tx, session: Mutex::new(SessionInfo::default()) });
    state.conns.lock().await.insert(label.clone(), handle);
    state.emit(Event::Connected { label: label.clone(), addr });
    run_connection(state, label, ws, outbound_rx).await;
}

async fn run_connection(
    state: Arc<AppState>,
    label: String,
    mut ws: WebSocketStream<TcpStream>,
    mut outbound_rx: mpsc::UnboundedReceiver<WsMessage>,
) {
    loop {
        tokio::select! {
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(msg) => { if ws.send(msg).await.is_err() { break; } }
                    None => break,
                }
            }
            incoming = ws.next() => {
                match incoming {
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        match proto::PeerMessage::decode(bytes.as_ref()) {
                            Ok(message) => {
                                auto_capture(&state, &label, &message).await;
                                state.emit(Event::Message { label: label.clone(), message });
                            }
                            Err(e) => state.emit(Event::Warn(format!("undecodable frame from {label}: {e}"))),
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => { state.emit(Event::Warn(format!("read error from {label}: {e}"))); break; }
                }
            }
        }
    }
    let _ = ws.close(None).await;
    state.conns.lock().await.remove(&label);
    state.emit(Event::Disconnected { label });
}

/// Convenience only — lets `send pair-challenge-response` etc. work off the
/// last-received nonce/peer key without you re-typing hex by hand. Doesn't
/// gate anything; you can still send any message at any time regardless.
async fn auto_capture(state: &Arc<AppState>, label: &str, message: &proto::PeerMessage) {
    let Some(handle) = state.conns.lock().await.get(label).cloned() else { return };
    let mut sess = handle.session.lock().await;
    match &message.body {
        Some(Body::PairRequest(r)) => {
            if let Ok(pk) = <[u8; 32]>::try_from(r.public_key.as_slice()) { sess.peer_public_key = Some(pk); }
            sess.peer_name = Some(r.name.clone());
        }
        Some(Body::PairResponse(r)) => {
            if let Ok(pk) = <[u8; 32]>::try_from(r.public_key.as_slice()) { sess.peer_public_key = Some(pk); }
            sess.peer_name = Some(r.name.clone());
        }
        Some(Body::PairChallenge(c)) => {
            if let Ok(n) = <[u8; 32]>::try_from(c.nonce.as_slice()) { sess.nonce = Some(n); }
        }
        Some(Body::ConnectChallenge(c)) => {
            if let Ok(n) = <[u8; 32]>::try_from(c.nonce.as_slice()) { sess.nonce = Some(n); }
        }
        _ => {}
    }
}

pub async fn send_body(state: &Arc<AppState>, label: &str, body: Body) -> anyhow::Result<()> {
    let handle = {
        let conns = state.conns.lock().await;
        conns.get(label).cloned().ok_or_else(|| anyhow::anyhow!("no such connection: {label}"))?
    };
    let message = proto::PeerMessage { body: Some(body) };
    let mut buf = Vec::new();
    message.encode(&mut buf)?;
    handle.outbound_tx.send(WsMessage::binary(buf)).map_err(|_| anyhow::anyhow!("connection closed"))
}
