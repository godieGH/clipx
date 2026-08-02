pub mod config;
pub mod manager;
pub mod seen;
pub mod trusted;
pub mod types;
pub mod pairing;

// some tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, IpAddr, Ipv4Addr};
    use std::time::Duration;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9999)
    }

    #[test]
    fn trusted_device_store_roundtrip() {
        let path = std::env::temp_dir().join("clipx_test_trusted.json");
        let _ = std::fs::remove_file(&path); // clean slate

        let mut store = trusted::TrustedDeviceStore::load(path.clone());
        assert!(!store.is_trusted("abc"));

        store.trust(types::TrustedDevice {
            id: "abc".into(),
            name: "Test Device".into(),
            device_type: types::DeviceType::Windows,
            paired_at: std::time::SystemTime::now(),
        });
        assert!(store.is_trusted("abc"));

        // reload from disk, confirm it persisted
        let reloaded = trusted::TrustedDeviceStore::load(path.clone());
        assert!(reloaded.is_trusted("abc"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn seen_registry_prunes_stale_devices() {
        let mut registry = seen::SeenDeviceRegistry::new();
        registry.upsert(types::SeenDevice {
            id: "abc".into(),
            name: "Test".into(),
            device_type: types::DeviceType::Windows,
            addr: test_addr(),
            last_seen: std::time::Instant::now() - Duration::from_secs(60),
        });

        registry.prune_stale(Duration::from_secs(30));
        assert!(registry.get("abc").is_none());
    }
}