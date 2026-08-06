use super::identity::{self, DeviceIdentity};
use super::trusted::TrustedDeviceStore;
use super::types::{SeenDevice, TrustedDevice};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingChallenge {
    pub request_id: String,
    pub nonce: Vec<u8>,
    pub initiator_public: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingChallengeResponse {
    pub request_id: String,
    pub signature: Vec<u8>,
    pub responder_public: Vec<u8>,
}

pub fn create_challenge(identity: &DeviceIdentity, initiator_public: &[u8]) -> PairingChallenge {
    let request_id = uuid::Uuid::new_v4().to_string();
    let nonce = identity.random_nonce();
    PairingChallenge {
        request_id: request_id.clone(),
        nonce: nonce.clone(),
        initiator_public: initiator_public.to_vec(),
    }
}

pub fn respond_to_challenge(
    identity: &DeviceIdentity,
    challenge: &PairingChallenge,
) -> PairingChallengeResponse {
    let signature = identity.sign(&challenge.nonce);
    PairingChallengeResponse {
        request_id: challenge.request_id.clone(),
        signature: signature.to_vec(),
        responder_public: identity.public_key_bytes().to_vec(),
    }
}

pub fn verify_response(
    challenge: &PairingChallenge,
    response: &PairingChallengeResponse,
    peer_public: &[u8],
) -> bool {
    identity::verify(peer_public, &challenge.nonce, &response.signature)
}

pub fn pairing_code(challenge: &PairingChallenge, response: &PairingChallengeResponse) -> String {
    let mut hasher = Sha256::new();
    hasher.update(&challenge.initiator_public);
    hasher.update(&response.responder_public);
    hasher.update(&challenge.nonce);
    let digest = hasher.finalize();
    let code = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 1000000;
    format!("{code:06}")
}

pub fn approve(seen: &SeenDevice, trusted: &mut TrustedDeviceStore) {
    trusted.trust(TrustedDevice {
        id: seen.id.clone(),
        name: seen.name.clone(),
        device_type: seen.device_type,
        paired_at: SystemTime::now(),
        public_key: [0u8; 32],
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_response_and_code_are_consistent() {
        let initiator = DeviceIdentity::load_or_create(std::env::temp_dir().join("clipx_test_pairing_initiator"));
        let responder = DeviceIdentity::load_or_create(std::env::temp_dir().join("clipx_test_pairing_responder"));
        let challenge = create_challenge(&initiator, &initiator.public_key_bytes());
        let response = respond_to_challenge(&responder, &challenge);

        assert!(verify_response(&challenge, &response, &responder.public_key_bytes()));
        let code = pairing_code(&challenge, &response);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
