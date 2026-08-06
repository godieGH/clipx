use clipx_core::device::{identity::DeviceIdentity, pairing};
use std::{env, path::PathBuf};

#[test]
fn pairing_handshake_generates_six_digit_code() {
    let temp_dir = env::temp_dir().join("clipx-test-pairing");
    let initiator = DeviceIdentity::load_or_create(temp_dir.join("initiator"));
    let responder = DeviceIdentity::load_or_create(temp_dir.join("responder"));

    let challenge = pairing::create_challenge(&initiator, &initiator.public_key_bytes());
    let response = pairing::respond_to_challenge(&responder, &challenge);

    assert!(pairing::verify_response(&challenge, &response, &responder.public_key_bytes()));

    let code = pairing::pairing_code(&challenge, &response);
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()));
}
