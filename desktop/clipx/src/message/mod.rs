use crate::clipx;

pub mod types {
    use serde::{Deserialize, Serialize};

    #[allow(non_snake_case)]
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct OwnIdentity {
        pub name: String,
        pub fingerprint: String,
        pub device_type: String,
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

    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ClipHistoryEntry {
        pub id: String,
        pub content: String,
        pub source_device: String,
        pub received_at: u64,
        pub kind: String,
        pub html: Option<String>,
        pub file_id: Option<String>,
        pub file_name: Option<String>,
        pub mime_type: Option<String>,
        pub file_size: u64,
        pub file_expires_at_ms: u64,
        pub file_downloaded: bool,
        pub local_file_path: Option<String>,
    }

    impl From<crate::clipx::ClipHistoryEntry> for ClipHistoryEntry {
        fn from(e: crate::clipx::ClipHistoryEntry) -> Self {
            Self {
                id: e.id,
                content: e.content,
                source_device: e.source_device_name,
                received_at: e.received_at_ms,
                kind: e.kind,
                html: if e.html.is_empty() {
                    None
                } else {
                    Some(e.html)
                },
                file_id: if e.file_id.is_empty() {
                    None
                } else {
                    Some(e.file_id)
                },
                file_name: if e.file_name.is_empty() {
                    None
                } else {
                    Some(e.file_name)
                },
                mime_type: if e.mime_type.is_empty() {
                    None
                } else {
                    Some(e.mime_type)
                },
                file_size: e.file_size,
                file_expires_at_ms: e.file_expires_at_ms,
                file_downloaded: e.file_downloaded,
                local_file_path: if e.local_file_path.is_empty() {
                    None
                } else {
                    Some(e.local_file_path)
                },
            }
        }
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

    impl From<crate::clipx::DeviceType> for String {
        fn from(t: crate::clipx::DeviceType) -> String {
            device_type_str(t as i32)
        }
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

pub trait IpcCmdBridge {
    fn get_this_device_identity(
        &mut self,
    ) -> impl std::future::Future<Output = Result<types::OwnIdentity, String>> + Send;
    fn get_paired_devices(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<types::PairedDevice>, String>> + Send;
    fn get_available_devices(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<types::AvailableDevice>, String>> + Send;
    fn connect_device(
        &mut self,
        device_id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn disconnect_device(
        &mut self,
        device_id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn pair_device(
        &mut self,
        device_id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn set_auto_connect(
        &mut self,
        device_id: String,
        auto_connect: bool,
    ) -> impl std::future::Future<Output = Result<bool, String>> + Send;
    fn forget_device(
        &mut self,
        device_id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn get_clipboard_history(
        &mut self,
        limit: Option<u32>,
    ) -> impl std::future::Future<Output = Result<Vec<types::ClipHistoryEntry>, String>> + Send;
    fn remove_clipboard_entry(
        &mut self,
        id: String,
    ) -> impl std::future::Future<Output = Result<bool, String>> + Send;
    fn clear_clipboard_history(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn send_text(
        &mut self,
        content: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn send_rich_text(
        &mut self,
        text: String,
        html: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn send_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn send_file(
        &mut self,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn send_file_path(
        &mut self,
        name: String,
        mime_type: String,
        path: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn download_clipboard_file(
        &mut self,
        id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
}

impl IpcCmdBridge for crate::ipc::non_blocking::Client {
    async fn get_this_device_identity(&mut self) -> Result<types::OwnIdentity, String> {
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
                    device_type: String::from(device_type),
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
            request: Some(clipx::ipc_request::Request::GetPaired(
                clipx::GetPairedRequest {},
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to fetch paired devices: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::GetPaired(r)) => Ok(r
                .devices
                .into_iter()
                .map(types::PairedDevice::from)
                .collect()),
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
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to fetch available devices: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Seen(r)) => Ok(r
                .devices
                .into_iter()
                .map(types::AvailableDevice::from)
                .collect()),
            _ => Err("Failed to fetch available devices: unexpected response variant".into()),
        }
    }

    async fn connect_device(&mut self, device_id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::Connect(
                clipx::ConnectRequest { device_id },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to connect device: {}", e))?;
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
            request: Some(clipx::ipc_request::Request::Disconnect(
                clipx::DisconnectRequest { device_id },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to disconnect device: {}", e))?;
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
            request: Some(clipx::ipc_request::Request::Pair(clipx::PairRequest {
                device_id,
            })),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to pair device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::Pair(r)) => Ok(r.message),
            _ => Err("Failed to pair device: unexpected response variant".into()),
        }
    }

    async fn set_auto_connect(
        &mut self,
        device_id: String,
        auto_connect: bool,
    ) -> Result<bool, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SetAutoConnect(
                clipx::SetAutoConnectRequest {
                    device_id,
                    auto_connect,
                },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to update auto-connect: {}", e))?;
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
            request: Some(clipx::ipc_request::Request::ForgetDevice(
                clipx::ForgetDeviceRequest { device_id },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to forget device: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::ForgetDevice(r)) => Ok(r.status),
            _ => Err("Failed to forget device: unexpected response variant".into()),
        }
    }
    async fn get_clipboard_history(
        &mut self,
        limit: Option<u32>,
    ) -> Result<Vec<types::ClipHistoryEntry>, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::ClipboardHistory(
                clipx::ClipboardHistoryRequest {
                    limit: limit.unwrap_or(0),
                },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to fetch clipboard history: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::ClipboardHistory(r)) => Ok(r
                .entries
                .into_iter()
                .map(types::ClipHistoryEntry::from)
                .collect()),
            _ => Err("Failed to fetch clipboard history: unexpected response variant".into()),
        }
    }

    async fn remove_clipboard_entry(&mut self, id: String) -> Result<bool, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::ClipboardRemove(
                clipx::ClipboardRemoveRequest { id },
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to remove clipboard entry: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::ClipboardRemove(r)) => Ok(r.removed),
            _ => Err("Failed to remove clipboard entry: unexpected response variant".into()),
        }
    }

    async fn clear_clipboard_history(&mut self) -> Result<(), String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::ClipboardClear(
                clipx::ClipboardClearRequest {},
            )),
        };
        let res = self
            .send(req)
            .await
            .map_err(|e| format!("Failed to clear clipboard history: {}", e))?;
        match res.response {
            Some(clipx::ipc_response::Response::ClipboardClear(_)) => Ok(()),
            _ => Err("Failed to clear clipboard history: unexpected response variant".into()),
        }
    }
    async fn send_text(&mut self, content: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SendText(
                clipx::SendTextRequest { content },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::Send(r)) if r.sent => Ok(r.message),
            Some(clipx::ipc_response::Response::Send(r)) => Err(r.message),
            _ => Err("Unexpected send response".into()),
        }
    }
    async fn send_rich_text(&mut self, text: String, html: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SendRichText(
                clipx::SendRichTextRequest { text, html },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::Send(r)) if r.sent => Ok(r.message),
            Some(clipx::ipc_response::Response::Send(r)) => Err(r.message),
            _ => Err("Unexpected send response".into()),
        }
    }
    async fn send_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SendImage(
                clipx::SendImageRequest {
                    width,
                    height,
                    rgba,
                },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::Send(r)) if r.sent => Ok(r.message),
            Some(clipx::ipc_response::Response::Send(r)) => Err(r.message),
            _ => Err("Unexpected send response".into()),
        }
    }
    async fn send_file(
        &mut self,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    ) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SendFile(
                clipx::SendFileRequest {
                    name,
                    mime_type,
                    data,
                },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::Send(r)) if r.sent => Ok(r.message),
            Some(clipx::ipc_response::Response::Send(r)) => Err(r.message),
            _ => Err("Unexpected send response".into()),
        }
    }
    async fn send_file_path(
        &mut self,
        name: String,
        mime_type: String,
        path: String,
    ) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::SendFilePath(
                clipx::SendFilePathRequest {
                    name,
                    mime_type,
                    path,
                },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::Send(r)) if r.sent => Ok(r.message),
            Some(clipx::ipc_response::Response::Send(r)) => Err(r.message),
            _ => Err("Unexpected send response".into()),
        }
    }
    async fn download_clipboard_file(&mut self, id: String) -> Result<String, String> {
        if !self.is_running() {
            self.start().await?;
        }
        let req = clipx::IpcRequest {
            request: Some(clipx::ipc_request::Request::DownloadClipboardFile(
                clipx::DownloadClipboardFileRequest { entry_id: id },
            )),
        };
        let res = self.send(req).await.map_err(|e| e.to_string())?;
        match res.response {
            Some(clipx::ipc_response::Response::DownloadClipboardFile(r)) if r.started => {
                Ok(r.message)
            }
            Some(clipx::ipc_response::Response::DownloadClipboardFile(r)) => Err(r.message),
            _ => Err("Unexpected download response".into()),
        }
    }
}
