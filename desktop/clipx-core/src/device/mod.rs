//! Device and pairing state for the clipboard-sharing mesh.
//!
//! This module groups identity, trust, pairing, and configuration logic used when
//! peers discover each other and negotiate a trusted connection.

/// Persistent configuration for each device instance.
pub mod config;
/// Local identity and fingerprint generation for a node.
pub mod identity;
/// Runtime device manager and command handling.
pub mod manager;
/// Pairing flow state and negotiation logic.
pub mod pairing;
/// "Seen" tracking for recent peer interactions.
pub mod seen;
/// Trusted device allow-list management.
pub mod trusted;
/// Shared device metadata structures.
pub mod types;
