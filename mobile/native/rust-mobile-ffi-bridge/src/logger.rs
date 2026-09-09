#![allow(unused)]

use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

use std::sync::Once;

static INIT: Once = Once::new();

#[uniffi::export]
pub fn init_logging() {
    INIT.call_once(|| {
        #[cfg(target_os = "android")]
        {
            let bridge_filter = Targets::new().with_target("rust_mobile_ffi_bridge", Level::TRACE);
            let core_filter = Targets::new().with_target("clipx_core", Level::TRACE);

            tracing_subscriber::registry()
                .with(
                    paranoid_android::layer("rust-mobile-ffi-bridge")
                        .with_ansi(false)
                        .with_filter(bridge_filter),
                )
                .with(
                    paranoid_android::layer("clipx-core")
                        .with_ansi(false)
                        .with_filter(core_filter),
                )
                .init();
        }

        #[cfg(target_os = "ios")]
        {
            // could use the oslog crate, or tracing-oslog layer when we get there
            todo!("Not implemented yet")
        }
    });
}
