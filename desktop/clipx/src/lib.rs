pub mod message;
pub mod ipc;
pub mod clipx {
    #![allow(unused)]
    mod clipx {
        include!(concat!(env!("OUT_DIR"), "/clipx.rs"));
    }
    
    pub use clipx::*;
    
}




/// the library also provide some public utility helper
pub mod utils {
    #![allow(unused)]
    use serde::{Deserialize, Serialize, de::DeserializeOwned};

    pub fn serialize(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }

    pub fn deserialize<T>(value: &str) -> anyhow::Result<T> 
    where T: DeserializeOwned {
        Ok(serde_json::from_str(value)?)
    }
}