use crate::{
    clipboard::manager::{ClipboardManager, ClipboardOutbound},
    device, netio,
    platform::{ClipboardSink, CoreEvent, CoreEventListener},
};

/// Runtime lifecycle state for the core clipboard service.
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use crate::platform::ArboardClipboardSink;
use device::identity::DeviceIdentity;
use std::sync::Arc;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tokio::signal;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServiceState {
    #[default]
    Stopped,
    Running,
    Stopping,
}

/// Central coordinator for the desktop clipboard engine.
///
/// The service owns the main task graph used for clipboard watching, discovery,
/// device coordination, and shutdown orchestration.
pub struct CoreService {
    state: ServiceState,
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    shutdown_tx: Option<watch::Sender<bool>>,
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    tasks: Vec<JoinHandle<()>>,
}

impl Default for CoreService {
    fn default() -> Self {
        Self {
            state: ServiceState::Stopped,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            shutdown_tx: None,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            tasks: vec![],
        }
    }
}

impl CoreService {
    /// Creates a new service in the stopped state.
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(unused)]
    /// Returns the latest lifecycle state of the service.
    pub fn state(&self) -> ServiceState {
        self.state
    }

    /// Marks the service as running and activates the internal task graph.
    pub fn start(&mut self) {
        self.state = ServiceState::Running;
    }

    /// Requests a graceful shutdown transition.
    pub fn stop(&mut self) {
        self.state = ServiceState::Stopping;
    }

    /// Resets the service back to the stopped state after shutdown completes.
    pub fn finish_stop(&mut self) {
        self.state = ServiceState::Stopped;
    }

    /// Starts the core, creates and manage all core tasks
    /// This is only available on desktop builds — it is the entry point to make
    /// the entire core up and running
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub async fn run(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.start();
        tracing::info!("Core service starting");

        let clipboard_sink = ArboardClipboardSink::new();
        let notification_engine = crate::notification::NotificationEngine::new();
        let (tasks, shutdown_tx, clipboard_cmd_tx, device_tx, core_events_tx) =
            spawn_core_tasks(clipboard_sink, notification_engine.clone(), None);
        self.tasks = tasks;
        self.shutdown_tx = Some(shutdown_tx.clone());

        let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();

        let mut ipc_service = netio::ipc::IpcService::new(
            "clipx",
            shutdown_tx.subscribe(),
            device_tx,
            clipboard_cmd_tx,
            core_events_tx,
            stop_tx,
        );
        self.tasks
            .push(tokio::spawn(async move { ipc_service.start().await }));

        tokio::select! {
            r = signal::ctrl_c() => { r?; }
            _ = stop_rx.recv() => {
                tracing::info!("Shutdown requested over IPC");
                // let the Shutdown reply flush before the sockets close
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }

        self.stop();
        self.shutdown().await;
        Ok(())
    }

    /// Desktops use this as shutdown lifecycle hook
    ///
    /// Ensures that all other parts of the core are completed; The `CoreService`
    /// broadcasts a shutdown signal to all other tasks/services
    /// and waits for them to wrap it up, clean or release resources for a graceful shutdown
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
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

/// Constructs the main task graph used by the clipboard runtime.
///
/// This helper wires clipboard observation, peer discovery, device management,
/// transport, and event forwarding into a single set of Tokio tasks.
pub fn spawn_core_tasks<S, N>(
    clipboard_sink: S,
    notification_engine: N,
    core_events: Option<Arc<dyn CoreEventListener>>,
) -> (
    Vec<JoinHandle<()>>,
    watch::Sender<bool>,
    mpsc::UnboundedSender<crate::clipboard::manager::ClipboardCommand>,
    mpsc::UnboundedSender<crate::device::manager::DeviceCommands>,
    broadcast::Sender<CoreEvent>,
)
where
    S: ClipboardSink + 'static,
    N: crate::notification::platform::NotificationEngine + Clone + Send + Sync + 'static,
{
    let mut tasks = Vec::<JoinHandle<()>>::new();
    let identity = Arc::new(DeviceIdentity::load_or_create(
        device::config::identity_key_path(),
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let (clipboard_cmd_tx, clipboard_cmd_rx) = mpsc::unbounded_channel();
    let (clipboard_out_tx, clipboard_out_rx) = mpsc::unbounded_channel::<ClipboardOutbound>();
    let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();
    let (device_tx, device_rx) = mpsc::unbounded_channel();
    let (transport_tx, transport_rx) = mpsc::unbounded_channel();
    let (peer_event_tx, peer_event_rx) = mpsc::unbounded_channel();
    let (core_events_tx, _) = broadcast::channel::<CoreEvent>(32);

    let shutdown_for_transport = shutdown_rx.clone();
    tasks.push(tokio::spawn(async move {
        let transport = crate::netio::transport::Transport::create_transport(
            shutdown_for_transport,
            transport_rx,
            peer_event_tx,
        )
        .await;
        transport.run().await;
    }));

    const MAX_CLIPBOARD_HISTORY: usize = 200;
    let clipboard_manager = ClipboardManager::new(
        device::config::clipboard_history_path(),
        MAX_CLIPBOARD_HISTORY,
        notification_engine.clone(),
        clipboard_sink,
        clipboard_out_tx,
        core_events_tx.clone(),
    );
    let shutdown_for_clipboard = shutdown_rx.clone();
    tasks.push(tokio::spawn(async move {
        clipboard_manager
            .run(shutdown_for_clipboard, clipboard_cmd_rx)
            .await;
    }));

    let shutdown_for_discovery = shutdown_rx.clone();
    let discovery_tasks =
        netio::discovery::spawn(shutdown_for_discovery, identity.clone(), discovered_tx)
            .expect("failed to start discovery");
    tasks.extend(discovery_tasks);

    #[cfg(windows)]
    tasks.push(netio::p2p_windows::spawn(
        shutdown_rx.clone(),
        hex::encode(identity.get_this_device_fingerprint()),
        device_tx.clone(),
    ));

    let device_manager = crate::device::manager::DeviceManager::new(
        device::config::trusted_devices_path(),
        identity,
        Some(transport_tx),
        notification_engine,
        clipboard_cmd_tx.clone(),
        core_events_tx.clone(),
    );
    let shutdown_for_device_manager = shutdown_rx.clone();
    let clipboard_cmd_tx_for_device_manager = clipboard_cmd_tx.clone();
    tasks.push(tokio::spawn(async move {
        device_manager
            .run(
                shutdown_for_device_manager,
                discovered_rx,
                device_rx,
                peer_event_rx,
                clipboard_out_rx,
                clipboard_cmd_tx_for_device_manager.clone(),
            )
            .await;
    }));

    if let Some(events) = core_events {
        let mut events_rx = core_events_tx.subscribe();
        let mut shutdown_for_events = shutdown_rx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_for_events.changed() => {
                        if *shutdown_for_events.borrow() { break; }
                    }
                    result = events_rx.recv() => {
                        match result {
                            Ok(event) => match event {
                                crate::platform::CoreEvent::DevicesChanged => events.on_device_change(),
                                crate::platform::CoreEvent::ClipboardChanged => events.on_clipboard_change(),
                                crate::platform::CoreEvent::PairingChanged { device_id, state, message } => {
                                    let state = match state {
                                        crate::platform::PairingEventState::Started => 0,
                                        crate::platform::PairingEventState::Failed => 1,
                                        crate::platform::PairingEventState::Succeeded => 2,
                                    };
                                    events.on_pairing_change(device_id, state, message);
                                }
                                crate::platform::CoreEvent::FileTransferChanged { entry_id, file_id, file_name, direction, done, total, state, message } => {
                                    events.on_file_transfer(entry_id, file_id, file_name, direction, done, total, state, message);
                                }
                            },
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        }));
    }

    (
        tasks,
        shutdown_tx,
        clipboard_cmd_tx,
        device_tx,
        core_events_tx,
    )
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
