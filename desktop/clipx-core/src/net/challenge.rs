// use crate::device::identity::{self, DeviceIdentity};
// use crate::message::proto;
// use crate::net::pending_requests::PendingRequests;
// use prost::Message;
// use std::net::SocketAddr;
// use std::sync::Arc;
// use std::time::Duration;
// use tokio::net::UdpSocket;
// use uuid::Uuid;

// pub async fn verify_peer(
//     socket: &UdpSocket,
//     pending: &PendingRequests<proto::ChallengeResponse>,
//     identity: &Arc<DeviceIdentity>,
//     peer_addr: SocketAddr,
//     peer_public_key: [u8; 32],
// ) -> bool {
//     let request_id = Uuid::new_v4().to_string();
//     let nonce = identity.random_nonce(); // see note below

//     let envelope = proto::UdpEnvelope {
//         payload: Some(proto::udp_envelope::Payload::Challenge(proto::Challenge {
//             request_id: request_id.clone(),
//             nonce: nonce.clone(),
//         })),
//     };
//     let mut buf = Vec::new();
//     envelope.encode(&mut buf).unwrap();

//     let rx = pending.wait_for(request_id.clone()).await;

//     if socket.send_to(&buf, peer_addr).await.is_err() {
//         pending.cancel(&request_id).await;
//         return false;
//     }

//     let response = match tokio::time::timeout(Duration::from_secs(2), rx).await {
//         Ok(Ok(response)) => response,
//         _ => {
//             pending.cancel(&request_id).await;
//             tracing::warn!("challenge to {peer_addr} timed out or channel dropped");
//             return false;
//         }
//     };

//     identity::verify(&peer_public_key, &nonce, &response.signature)
// }