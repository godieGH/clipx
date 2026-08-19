use crate::{clipboard::manager::ClipboardManager, device, netio};
use device::identity::DeviceIdentity;
use std::sync::Arc;
use tokio::{
    signal,
    sync::{mpsc, watch},
    task::JoinHandle,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServiceState {
    #[default]
    Stopped,
    Running,
    Stopping,
}

pub struct CoreService {
    state: ServiceState,
    shutdown_tx: Option<watch::Sender<bool>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Default for CoreService {
    fn default() -> Self {
        Self {
            state: ServiceState::Stopped,
            shutdown_tx: None,
            tasks: vec![],
        }
    }
}

impl CoreService {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(unused)]
    pub fn state(&self) -> ServiceState {
        self.state
    }

    pub fn start(&mut self) {
        self.state = ServiceState::Running;
    }

    pub fn stop(&mut self) {
        self.state = ServiceState::Stopping;
    }

    pub fn finish_stop(&mut self) {
        self.state = ServiceState::Stopped;
    }

    pub async fn run(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.start();
        tracing::info!("Core service starting");

        let identity = Arc::new(DeviceIdentity::load_or_create(
            device::config::identity_key_path(),
        ));

        let notification_engine = crate::notification::NotificationEngine::new();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        // clipboard_cmd_*: how anyone (Device Manager, the IPC layer)
        // talks to the clipboard component.
        // clipboard_out_*: how the clipboard component hands local changes
        // to the Device Manager for dispatch — one direction, one way.
        let (clipboard_cmd_tx, clipboard_cmd_rx) = mpsc::unbounded_channel();
        let (clipboard_out_tx, clipboard_out_rx) = mpsc::unbounded_channel();
        let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();
        let (device_tx, device_rx) = mpsc::unbounded_channel();
        let (transport_tx, transport_rx) = mpsc::unbounded_channel();
        let (peer_event_tx, peer_event_rx) = mpsc::unbounded_channel();

        let shutdown_for_transport = shutdown_rx.clone();
        let transport_task = tokio::spawn(async move {
            let transport = crate::netio::transport::Transport::create_transport(
                shutdown_for_transport,
                transport_rx,
                peer_event_tx,
            )
            .await;
            transport.run().await;
        });
        self.tasks.push(transport_task);

        let shutdown_for_device_manager = shutdown_rx.clone();
        let shutdown_for_clipboard = shutdown_rx.clone();
        let shutdown_for_discovery = shutdown_rx.clone();
        let shutdown_for_ipc = shutdown_rx.clone();

        const MAX_CLIPBOARD_HISTORY: usize = 200;
        let clipboard_manager = ClipboardManager::new(
            device::config::clipboard_history_path(),
            MAX_CLIPBOARD_HISTORY,
            notification_engine.clone(),
            clipboard_out_tx,
        );
        let clipboard_task = tokio::spawn(async move {
            clipboard_manager
                .run(shutdown_for_clipboard, clipboard_cmd_rx)
                .await;
        });
        self.tasks.push(clipboard_task);

        let discovery_tasks =
            netio::discovery::spawn(shutdown_for_discovery, identity.clone(), discovered_tx)
                .expect("failed to start discovery");
        self.tasks.extend(discovery_tasks);

        let device_manager = device::manager::DeviceManager::new(
            device::config::trusted_devices_path(),
            identity.clone(),
            Some(transport_tx),
            notification_engine.clone(),
            clipboard_cmd_tx.clone(),
        );
        let device_manager_task = tokio::spawn(async move {
            device_manager
                .run(
                    shutdown_for_device_manager,
                    discovered_rx,
                    device_rx,
                    peer_event_rx,
                    clipboard_out_rx,
                )
                .await;
        });
        self.tasks.push(device_manager_task);

        // Same command channel the Device Manager uses — the IPC layer can
        // issue GetHistory/RemoveEntry/ClearHistory the same way it issues
        // DeviceCommands, once those routes are added there.
        let mut ipc_service = netio::ipc::IpcService::new(
            "clipx",
            shutdown_for_ipc,
            device_tx.clone(),
            clipboard_cmd_tx.clone(),
        );
        let ipc_task = tokio::spawn(async move { ipc_service.start().await });
        self.tasks.push(ipc_task);

        signal::ctrl_c().await?;

        self.stop();
        self.shutdown().await;
        Ok(())
    }

    async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        for task in self.tasks.drain(..) {
            let _ = task.await;
        }

        self.finish_stop();
        tracing::info!("Core service stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_state_transitions_cleanly() {
        let mut service = CoreService::new();

        assert_eq!(service.state(), ServiceState::Stopped);

        service.start();
        assert_eq!(service.state(), ServiceState::Running);

        service.stop();
        assert_eq!(service.state(), ServiceState::Stopping);

        service.finish_stop();
        assert_eq!(service.state(), ServiceState::Stopped);
    }
}
