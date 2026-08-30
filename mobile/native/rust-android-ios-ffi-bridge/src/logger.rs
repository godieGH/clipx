use android_logger::Config;
use log::LevelFilter;

// TODO: This should be os aware gated or made in a way it works crossly
// for now we can just leave it android support — until ios comes up
#[uniffi::export]
pub fn init_logging() {
    android_logger::init_once(
        Config::default()
            .with_max_level(LevelFilter::Trace)
            .with_tag("rust-android-ffi-bridge"),
    );
}
