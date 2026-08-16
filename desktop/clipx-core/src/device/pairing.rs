#![allow(unused)]
use std::{net::SocketAddr, time::Instant};

#[derive(Debug)]
pub struct PairSession {
    started_at: Instant,
    stage: PairStage,
    nounce: Option<[u8; 32]>,
    code: Option<u32>,
    addr: SocketAddr,
}

/// This is an alternating stage for both peer and initiator
/// If initiatore starts with Requesting then the next stage will be Challanging
/// While the peer will be take Responding — Signing — 
#[derive(Debug)]
pub enum PairStage {
    Starting, // first default stage
    Requesting,
    Responding,
    Challanging,
    Signing,
    Verifying, // this is final stag of the initiator
    Acknowledged // This is the final stage of the peer
}

impl PairSession {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            started_at: Instant::now(),
            stage: PairStage::Starting,
            nounce: None,
            code: None,
            addr,
        }
    }

    // other related methods 
    // this should be responsible for also creating message
}