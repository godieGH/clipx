use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
/// Holds this device's permanent signing keypair.
/// The private key never leaves this struct/this device.
pub struct DeviceIdentity {
    signing_key: SigningKey,
}

enum LoadError {
    Transient(std::io::Error),
    Fatal(String),
}

impl DeviceIdentity {
    /// Loads the persisted private key if one exists, otherwise generates
    /// a new keypair and persists the private key immediately.
    const READ_MAX_ATTEMPTS: usize = 5;
    const READ_RETRY_DELAY: Duration = Duration::from_millis(800);

    pub fn load_or_create(path: PathBuf) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match Self::try_load_or_create(&path) {
                Ok(s) => return Ok(s),
                Err(LoadError::Transient(e)) if attempt < Self::READ_MAX_ATTEMPTS => {
                    tracing::warn!(%e, attempt, "could not read device identity, retrying");
                    std::thread::sleep(Self::READ_RETRY_DELAY);
                }
                Err(LoadError::Transient(e)) => {
                    return Err(format!(
                        "failed to read device identity '{}' after {attempt} attempts: {e}",
                        path.display()
                    )
                    .into());
                }
                Err(LoadError::Fatal(msg)) => return Err(msg.into()),
            }
        }
    }

    fn try_load_or_create(path: &PathBuf) -> Result<Self, LoadError> {
        tracing::trace!("load or create persisted device identity");
        match fs::read(path) {
            Ok(bytes) => match <[u8; 32]>::try_from(bytes.as_slice()) {
                Ok(key) => {
                    tracing::debug!("loaded persisted device identity");
                    Ok(Self {
                        signing_key: SigningKey::from_bytes(&key),
                    })
                }
                Err(_) => {
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let mut name = path.file_name().unwrap_or_default().to_os_string();
                    name.push(format!(".corrupt-{ts}"));
                    let backup = path.with_file_name(name);
                    tracing::warn!("identity key file was malformed");

                    let moved = match fs::rename(path, &backup) {
                        Ok(()) => format!("moved to '{}'", backup.display()),
                        Err(e) => format!("could not be moved aside: {e}"),
                    };
                    Err(LoadError::Fatal(format!(
                        "device identity '{}' is malformed ({} bytes, expected 32); {moved}",
                        path.display(),
                        bytes.len()
                    )))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::trace!("no persisted device identity found; generating a new key");
                let signing_key = SigningKey::generate(&mut OsRng);

                let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
                tmp_name.push(format!(".tmp-{}", std::process::id()));
                let tmp = path.with_file_name(tmp_name);
                let _ = fs::remove_file(&tmp);

                let result = (|| -> std::io::Result<()> {
                    let mut f = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&tmp)?;
                    f.write_all(&signing_key.to_bytes())?;
                    f.sync_all()?;
                    fs::hard_link(&tmp, path) // atomic, fails if path already exists
                })();
                let _ = fs::remove_file(&tmp);

                match result {
                    Ok(()) => {
                        tracing::info!("generated a new device identity");
                        Ok(Self { signing_key })
                    }
                    // another instance created it first: retry and load theirs
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        Err(LoadError::Transient(e))
                    }
                    Err(e) => Err(LoadError::Fatal(format!(
                        "failed to create device identity '{}': {e}",
                        path.display()
                    ))),
                }
            }
            Err(e) => Err(LoadError::Transient(e)),
        }
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
    tracing::trace!(
        public_key_bytes = public_key_bytes.len(),
        message_bytes = message.len(),
        signature_bytes = signature_bytes.len(),
        "verifying peer signature"
    );
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
        assert!(identity.is_ok());
        let identity = identity.unwrap();
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
        assert!(identity.is_ok());
        let identity = identity.unwrap();
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
        assert!(identity_a.is_ok() && identity_b.is_ok());
        let (identity_a, identity_b) = (identity_a.unwrap(), identity_b.unwrap());

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
        assert!(identity1.is_ok());
        let identity1 = identity1.unwrap();
        let pubkey1 = identity1.public_key_bytes();

        let identity2 = DeviceIdentity::load_or_create(path.clone());
        assert!(identity2.is_ok());
        let identity2 = identity2.unwrap();
        let pubkey2 = identity2.public_key_bytes();

        assert_eq!(
            pubkey1, pubkey2,
            "reloading should return the same keypair, not a new one"
        );

        fs::remove_file(&path).ok();
    }
}
