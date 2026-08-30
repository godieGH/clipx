use android_logger::Config;
use log::LevelFilter;

#[uniffi::export]
pub fn init_logging() {
    android_logger::init_once(
        Config::default()
            .with_max_level(LevelFilter::Trace)
            .with_tag("rust-android-ffi-bridge"),
    );
}
