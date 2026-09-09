use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Holds this device's permanent signing keypair.
/// The private key never leaves this struct/this device.
pub struct DeviceIdentity {
    signing_key: SigningKey,
}

impl DeviceIdentity {
    /// Loads the persisted private key if one exists, otherwise generates
    /// a new keypair and persists the private key immediately.
    pub fn load_or_create(path: PathBuf) -> Self {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(key_bytes) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Self {
                    signing_key: SigningKey::from_bytes(&key_bytes),
                };
            }
            tracing::warn!("identity key file at {path:?} was malformed, regenerating");
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        if let Err(e) = fs::write(&path, signing_key.to_bytes()) {
            tracing::error!("failed to persist identity key: {e}");
        }
        Self { signing_key }
    }

    /// The public key to advertise in broadcasts and store for trusted peers.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Signs a message with this device's private key.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    pub fn random_nonce(&self) -> Vec<u8> {
        let mut nonce = vec![0u8; 32];
        OsRng.fill_bytes(&mut nonce);
        nonce
    }

    // gives us fingerprint for device id from the public key
    pub fn get_this_device_fingerprint(&self) -> [u8; 32] {
        let public_key_bytes = self.public_key_bytes();
        DeviceIdentity::get_fingerprint_for(public_key_bytes)
    }
}

impl DeviceIdentity {
    pub fn get_fingerprint_for(public_key: [u8; 32]) -> [u8; 32] {
        Sha256::digest(public_key).into()
    }
}

/// Verifies a signature against a claimed public key.
/// Returns false (never panics) on any malformed input — this handles
/// untrusted network bytes, so it must never crash the caller.
pub fn verify(public_key_bytes: &[u8], message: &[u8], signature_bytes: &[u8]) -> bool {
    let Ok(pk_array) = <[u8; 32]>::try_from(public_key_bytes) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
        return false;
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(signature_bytes) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);

    verifying_key.verify(message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_key_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("clipx_test_identity_{name}"))
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let path = temp_key_path("roundtrip");
        let _ = fs::remove_file(&path);

        let identity = DeviceIdentity::load_or_create(path.clone());
        let message = b"hello device";
        let signature = identity.sign(message);

        assert!(verify(&identity.public_key_bytes(), message, &signature));

        fs::remove_file(&path).ok();
    }

    #[test]
    fn tampered_message_fails_verification() {
        let path = temp_key_path("tampered");
        let _ = fs::remove_file(&path);

        let identity = DeviceIdentity::load_or_create(path.clone());
        let signature = identity.sign(b"original message");

        assert!(!verify(
            &identity.public_key_bytes(),
            b"different message",
            &signature
        ));

        fs::remove_file(&path).ok();
    }

    #[test]
    fn wrong_public_key_fails_verification() {
        let path_a = temp_key_path("wrong_key_a");
        let path_b = temp_key_path("wrong_key_b");
        let _ = fs::remove_file(&path_a);
        let _ = fs::remove_file(&path_b);

        let identity_a = DeviceIdentity::load_or_create(path_a.clone());
        let identity_b = DeviceIdentity::load_or_create(path_b.clone());

        let message = b"hello";
        let signature = identity_a.sign(message);

        // signed by A, but checked against B's public key — must fail
        assert!(!verify(&identity_b.public_key_bytes(), message, &signature));

        fs::remove_file(&path_a).ok();
        fs::remove_file(&path_b).ok();
    }

    #[test]
    fn identity_persists_across_reload() {
        let path = temp_key_path("persist");
        let _ = fs::remove_file(&path);

        let identity1 = DeviceIdentity::load_or_create(path.clone());
        let pubkey1 = identity1.public_key_bytes();

        let identity2 = DeviceIdentity::load_or_create(path.clone());
        let pubkey2 = identity2.public_key_bytes();

        assert_eq!(
            pubkey1, pubkey2,
            "reloading should return the same keypair, not a new one"
        );

        fs::remove_file(&path).ok();
    }
}
