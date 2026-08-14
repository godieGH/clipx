use crate::clipx;

pub mod types {
    use serde::{Deserialize, Serialize};

    #[allow(non_snake_case)]
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct OwnIdentity {
        pub name: String,
        pub fingerprint: String,
        pub device_type: crate::clipx::DeviceType,
        pub ws_port: u32,
        pub ip_address: String,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct PairedDevice {
        pub fingerprint: String,
        pub name: String,
        pub device_type: String,
        pub connection: String,
        pub ip_address: String,
        pub ws_port: u32,
        pub auto_connect: bool,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct AvailableDevice {
        pub fingerprint: String,
        pub name: String,
        pub device_type: String,
    }

    pub(super) fn device_type_str(t: i32) -> String {
        use crate::clipx::DeviceType::*;
        match crate::clipx::DeviceType::try_from(t).unwrap_or(Unspecified) {
            Unspecified => "unspecified",
            Windows => "windows",
            Android => "android",
            Linux => "linux",
            Macos => "macos",
            Ios => "ios",
        }
        .to_string()
    }

    pub(super) fn connection_str(c: i32) -> String {
        use crate::clipx::ConnectionState::*;
        match crate::clipx::ConnectionState::try_from(c).unwrap_or(Disconnected) {
            Disconnected => "disconnected",
            Connecting => "connecting",
            Connected => "connected",
            Unavailable => "unavailable",
        }
        .to_string()
    }

    impl From<crate::clipx::DeviceInfo> for PairedDevice {
        fn from(d: crate::clipx::DeviceInfo) -> Self {
            Self {
                fingerprint: d.id,
                name: d.name,
                device_type: device_type_str(d.device_type),
                connection: connection_str(d.connection),
                ip_address: d.address,
                ws_port: d.ws_port,
                auto_connect: d.auto_connect,
            }
        }
    }

    impl From<crate::clipx::DeviceInfo> for AvailableDevice {
        fn from(d: crate::clipx::DeviceInfo) -> Self {
            Self {
                fingerprint: d.id,
                name: d.name,
                device_type: device_type_str(d.device_type),
            }
        }
    }
}

pub trait IpcCmddBridge {
    fn get_this_device_identity(&mut self) -> impl std::future::Future<Output = Result<types::OwnIdentity, String>> + Send;
    fn get_paired_devices(&mut self) -> impl std::future::Future<Output = Result<Vec<types::PairedDevice>, String>> + Send;
    fn get_available_devices(&mut self) -> impl std::future::Future<Output = Result<Vec<types::AvailableDevice>, String>> + Send;
    fn connect_device(&mut self, device_id: String) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn disconnect_device(&mut self, device_id: String) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn pair_device(&mut self, device_id: String) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn set_auto_connect(&mut self, device_id: String, auto_connect: bool) -> impl std::future::Future<Output = Result<bool, String>> + Send;
    fn forget_device(&mut self, device_id: String) -> impl std::future::Future<Output = Result<String, String>> + Send;
}

impl IpcCmddBridge for crate::ipc::non_blocking::Client {
    async fn get_this_device_identity(&mut self) -> Result<types::OwnIdentity, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Identity(clipx::IdentityRequest {})),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to fetch device Identity: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Identity(response)) => {
                let device_type = clipx::DeviceType::try_from(response.device_type)
                    .map_err(|e| format!("Failed to fetch device Identity: {}", e))?;
                Ok(types::OwnIdentity {
                    name: response.device_name,
                    fingerprint: response.device_id,
                    device_type: device_type,
                    ws_port: response.ws_port,
                    ip_address: response.ip_address,
                })
            }
            _ => Err("Failed to fetch device Identity: unexpected response variant".into()),
        }
    }

    async fn get_paired_devices(&mut self) -> Result<Vec<types::PairedDevice>, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::GetPaired(clipx::GetPairedRequest {})),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to fetch paired devices: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::GetPaired(r)) => {
                Ok(r.devices.into_iter().map(types::PairedDevice::from).collect())
            }
            _ => Err("Failed to fetch paired devices: unexpected response variant".into()),
        }
    }

    async fn get_available_devices(&mut self) -> Result<Vec<types::AvailableDevice>, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Seen(clipx::SeenRequest {
                mode: clipx::seen_request::Mode::Untrusted as i32,
            })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to fetch available devices: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Seen(r)) => {
                Ok(r.devices.into_iter().map(types::AvailableDevice::from).collect())
            }
            _ => Err("Failed to fetch available devices: unexpected response variant".into()),
        }
    }

    async fn connect_device(&mut self, device_id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Connect(clipx::ConnectRequest { device_id })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to connect device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Connect(r)) => Ok(r.message),
            _ => Err("Failed to connect device: unexpected response variant".into()),
        }
    }

    async fn disconnect_device(&mut self, device_id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Disconnect(clipx::DisconnectRequest { device_id })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to disconnect device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Disconnect(r)) => Ok(r.message),
            _ => Err("Failed to disconnect device: unexpected response variant".into()),
        }
    }

    async fn pair_device(&mut self, device_id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest { device_id })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to pair device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Pair(r)) => Ok(r.message),
            _ => Err("Failed to pair device: unexpected response variant".into()),
        }
    }

    async fn set_auto_connect(&mut self, device_id: String, auto_connect: bool) -> Result<bool, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SetAutoConnect(clipx::SetAutoConnectRequest {
                device_id, auto_connect,
            })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to update auto-connect: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::SetAutoConnect(r)) => Ok(r.auto_connect),
            _ => Err("Failed to update auto-connect: unexpected response variant".into()),
        }
    }

    async fn forget_device(&mut self, device_id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::ForgetDevice(clipx::ForgetDeviceRequest { device_id })),
        };
        let res = self.send(req).await.map_err(|e| format!("Failed to forget device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::ForgetDevice(r)) => Ok(r.status),
            _ => Err("Failed to forget device: unexpected response variant".into()),
        }
    }
}