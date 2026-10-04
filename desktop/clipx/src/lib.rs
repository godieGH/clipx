pub mod ipc;
pub mod message;
pub mod clipx {
    #![allow(unused)]
    #[allow(clippy::module_inception)]
    mod clipx {
        include!(concat!(env!("OUT_DIR"), "/clipx.rs"));
    }

    pub use clipx::*;
}

/// the library also provide some public utility helper
pub mod utils {
    #![allow(unused)]
    use serde::{de::DeserializeOwned, Deserialize, Serialize};

    pub fn serialize(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }

    pub fn deserialize<T>(value: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        Ok(serde_json::from_str(value)?)
    }
}
