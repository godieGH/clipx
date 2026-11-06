use std::time::Instant;

use crate::message::proto::{self, peer_message::Body};

/// Identifies a file-transfer message across both devices.
///
/// A FileChunk uses its ending offset, so it matches the
/// confirmed_offset carried by the corresponding FileChunkAck.
#[derive(Debug, Clone)]
pub struct MessageTrace {
    pub message_type: &'static str,
    pub file_id: String,
    pub confirmed_offset: u64,
    pub seq: Option<u64>,
    pub payload_bytes: usize,
}

impl MessageTrace {
    pub fn from_message(message: &proto::PeerMessage) -> Option<Self> {
        match message.body.as_ref()? {
            Body::FileChunk(chunk) => Some(Self {
                message_type: "FileChunk",
                file_id: chunk.file_id.clone(),
                confirmed_offset: chunk.offset.saturating_add(chunk.data.len() as u64),
                seq: Some(chunk.seq),
                payload_bytes: chunk.data.len(),
            }),

            Body::FileChunkAck(ack) => Some(Self {
                message_type: "FileChunkAck",
                file_id: ack.file_id.clone(),
                confirmed_offset: ack.confirmed_offset,
                seq: None,
                payload_bytes: 0,
            }),

            _ => None,
        }
    }
}

/// Timing information attached to an inbound message event.
///
/// Transport records decoding and the time it enqueues the event.
/// DeviceManager measures how long the event waits before processing.
#[derive(Debug)]
pub struct InboundMessageProfile {
    pub trace: MessageTrace,
    pub decode_us: u64,
    pub event_enqueued_at: Instant,
}
