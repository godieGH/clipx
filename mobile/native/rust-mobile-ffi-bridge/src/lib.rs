//! This is the ffi bridge entry point — provides cross APIs bridging
//! clipx-core or any other reusable rust materials across platforms
//! It uses an `os-agnostic` conventions and exposes ffi standard APIs
//! this lib tries the best to bridge + reuse `clip-core::*` APIs wrap them and ensure
//! android/ios can reuse the parts eg. networking + device manager + data protocols + clipboard etc.
//! for example if — instead of using the arboard crate to watch clipboard states — this lib would create a bridge
//! that makes android clipboard services to notify the core about change in clipboard content
//! This lib can create and run the core service in a different manner not as it does in desktop — maybe
//! remove things like IPCs + notification engine (or reuse one but with delegation to the android notifications or UI)
//! all that are not in terms with android

#![allow(clippy::too_many_arguments)]

mod logger;
mod platform;
mod service;

uniffi::setup_scaffolding!();
