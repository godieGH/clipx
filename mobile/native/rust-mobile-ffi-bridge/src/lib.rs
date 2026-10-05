//! # rust-mobile-ffi-bridge
//!
//! Platform-neutral FFI layer used by the Android and iOS apps to access the
//! shared Clipx core. This crate adapts native clipboard and notification APIs
//! into the Rust interfaces expected by the desktop-focused core runtime.
//!
//! The bridge keeps the host code free from Rust details while exposing a small,
//! stable API surface for identity, pairing, clipboard sync, and event handling.

#![allow(clippy::too_many_arguments)]

mod logger;
mod platform;
mod service;

uniffi::setup_scaffolding!();
