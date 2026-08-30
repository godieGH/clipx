//! This is the ffi bridge entry point — provides cross APIs bridging
//! clipx-core or any other reusable rust materials across platforms
//! It uses an `os-agnostic` conventions and exposes ffi standard APIs

mod logger;

uniffi::setup_scaffolding!();

