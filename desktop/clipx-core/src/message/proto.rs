#![allow(unused)]

pub mod clipx {
    include!(concat!(env!("OUT_DIR"), "/clipx.rs"));
}

pub use clipx::*;

impl From<DeviceType> for String {
    fn from(t: DeviceType) -> String {
        match t {
            DeviceType::Android => "Android".to_string(),
            DeviceType::Ios => "Ios".to_string(),
            DeviceType::Linux => "Linux".to_string(),
            DeviceType::Macos => "Macos".to_string(),
            DeviceType::Unspecified => "Unkknow".to_string(),
            DeviceType::Windows => "Windows".to_string()
        }
    }
}
