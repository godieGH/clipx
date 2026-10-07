//! Networking and transport glue for peer discovery and data exchange.
//!
//! The network runtime is responsible for locating peers, maintaining socket
//! sessions, and relaying clipboard and device events between machines.

/// Peer discovery and local network announcements.
pub mod discovery;
/// Local IPC service used by desktop clients and the UI shell.
pub mod ipc;
/// Windows Wi-Fi Direct link: hosts a legacy group and joins other ClipX hosts.
#[cfg(windows)]
pub mod p2p_windows;
/// Connection transport and low-level socket handling.
pub mod transport;
