//! This module provide a messaging layer, Command Bridge it ensures callers of the IPC layer can just call into
//! Already made commands to the core without formulating new message protocols at their sides
//! A reusable IPC message protocol that all commands/library users should apply to send/receive messages
//! It uses the defined protobuf messaging interfaces to provide different high-lever APIs
//! to communicate over the IPC channel without knowing the underlying protocols.

use crate::clipx; // the protobuf message defns

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

/// Ipc can now inherit the bridge superpowers
impl IpcCmddBridge for crate::ipc::non_blocking::Client {
    async fn get_this_device_identity(&mut self) -> Result<types::OwnIdentity, String> {
        let ipcreq = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Identity(
                clipx::IdentityRequest {},
            )),
        };

        if let Ok(res) = self.send(ipcreq).await {
            if let Some(clipx::ipc_response::Response::Identity(response)) = res.response {
                let device_type = match clipx::DeviceType::try_from(response.device_type) {
                    Ok(res) => res,
                    Err(_) => return Err("Failed to fetch device Identity".into())
                };
                return Ok(types::OwnIdentity {
                    name: response.device_name,
                    fingerprint: response.device_id,
                    deviceType: device_type,
                    wsPort: response.ws_port,
                    ipAddress: response.ip_address,
                });
            }
        }
        
        Err("Failed to fetch device Identity".into())
    }
}
