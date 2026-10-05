//! # clipx-core
//!
//! Shared runtime for clipboard sync across desktop and mobile clients. The crate
//! owns discovery, peer management, network transport, clipboard mirroring, and
//! user-facing notification flows while keeping platform-specific integrations in
//! thin adapters.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

/// Clipboard synchronization and persistence logic.
pub mod clipboard;
/// Peer and device state management, pairing metadata, and trust configuration.
pub mod device;
/// Process-wide logging setup for the core service.
pub mod logging;
/// Wire-format and serialization bindings for inter-device messages.
pub mod message;
/// Discovery, networking, and IPC primitives for peer communication.
pub mod netio;
/// Notification prompts and transfer progress handling.
pub mod notification;
/// Host-platform abstractions consumed by the core runtime.
pub mod platform;
/// Main runtime coordinator that wires the core tasks together.
pub mod service;
