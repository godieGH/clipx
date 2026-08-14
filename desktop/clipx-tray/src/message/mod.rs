use crate::clipx;

pub mod types {
    use serde::{Deserialize, Serialize};

    #[allow(non_snake_case)]
    #[derive(Debug, Serialize, Deserialize)]
    pub struct OwnIdentity {
        pub name: String,
        pub fingerprint: String,
        pub deviceType: crate::clipx::DeviceType,
        pub wsPort: u32,
        pub ipAddress: String,
    }
}

pub trait IpcCmddBridge {
    fn get_this_device_identity(&mut self) -> impl std::future::Future<Output = Result<types::OwnIdentity, String>> + Send;
}

impl IpcCmddBridge for crate::ipc::non_blocking::Client {
    async fn get_this_device_identity(&mut self) -> Result<types::OwnIdentity, String> {
        // Self-heal: if a previous send() found the connection dead, reconnect
        // before trying again instead of failing forever.
        if !self.is_running() {
            self.start().await?;
        }

        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Identity(
                clipx::IdentityRequest {},
            )),
        };

        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to fetch device Identity: {}", e))?;

        match res.response {
            Some(clipx::ipc_response::Response::Identity(response)) => {
                let device_type = clipx::DeviceType::try_from(response.device_type)
                    .map_err(|e| format!("Failed to fetch device Identity: {}", e))?;
                Ok(types::OwnIdentity {
                    name: response.device_name,
                    fingerprint: response.device_id,
                    deviceType: device_type,
                    wsPort: response.ws_port,
                    ipAddress: response.ip_address,
                })
            }
            _ => Err("Failed to fetch device Identity: unexpected response variant".into()),
        }
    }
}