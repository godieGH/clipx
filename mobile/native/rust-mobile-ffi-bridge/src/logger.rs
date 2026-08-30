#[allow(unused)]
use log::LevelFilter;

// TODO: This should be os aware gated (internally so export seems agnostic) or made in a way it works crossly
// for now we can just leave it android support — until ios comes up
#[uniffi::export]
pub fn init_logging() {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(LevelFilter::Trace)
                .with_tag("rust-android-ffi-bridge"),
        );

        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        tracing_subscriber::registry()
            .with(paranoid_android::layer("clipx-core"))
            .init();
    }

    #[cfg(target_os = "ios")]
    {
        // could use the oslog crate, or tracing-oslog layer when we get there
        todo!("Not implemeted yet")
    }
}
