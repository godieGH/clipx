use crate::{clipboard::watcher, device, net};
use device::identity::DeviceIdentity;
use std::sync::Arc;
use tokio::{signal, sync::{mpsc, watch}, task::JoinHandle};

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

        let device_id = device::config::get_or_create_device_id(device::config::machine_device_id_path());
        let identity = Arc::new(DeviceIdentity::load_or_create(device::config::identity_key_path()));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();
        let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();

        let shutdown_for_device_manager = shutdown_rx.clone();
        let shutdown_for_clipboard = shutdown_rx.clone();
        let shutdown_for_transport = shutdown_rx.clone();
        let shutdown_for_discovery = shutdown_rx.clone();

        let watcher_task = tokio::spawn(async move {
            watcher::watch_clipboard(shutdown_for_clipboard, clipboard_tx).await;
        });
        self.tasks.push(watcher_task);

        let (discovery_tasks, shared_socket, pending) = net::discovery::spawn(
            shutdown_for_discovery,
            device_id.clone(),
            identity.clone(),
            discovered_tx,
        ).expect("failed to start discovery");
        self.tasks.extend(discovery_tasks);

        let device_manager = device::manager::DeviceManager::new(
            device::config::trusted_devices_path(),
            identity.clone(),
            shared_socket,
            pending,
        );
        let device_manager_task = tokio::spawn(async move {
            device_manager.run(shutdown_for_device_manager, discovered_rx).await;
        });
        self.tasks.push(device_manager_task);

        let transport_task = tokio::spawn(async move {
            net::transport::cordinator(shutdown_for_transport, clipboard_rx).await
        });
        self.tasks.push(transport_task);

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