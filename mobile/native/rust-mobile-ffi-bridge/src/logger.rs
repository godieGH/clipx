#![allow(unused)]
use std::{collections::VecDeque, fmt, sync::{Mutex, Once, OnceLock}, time::{SystemTime, UNIX_EPOCH}};
use tracing::{field::{Field, Visit}, Event, Level, Subscriber};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

static INIT: Once = Once::new();
const MAX_DIAGNOSTIC_LOGS: usize = 250;
static DIAGNOSTIC_LOGS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
fn log_buffer() -> &'static Mutex<VecDeque<String>> {
    DIAGNOSTIC_LOGS.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_DIAGNOSTIC_LOGS)))
}
#[derive(Default)]
struct EventFields { fields: Vec<String> }
impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let key = field.name().to_ascii_lowercase();
        let sensitive = ["clipboard","content","text","payload","path","ip","address","fingerprint","file_name","filename"].iter().any(|p| key.contains(p));
        let value = if sensitive { "<redacted>".to_string() } else { format!("{value:?}") };
        self.fields.push(format!("{}={}", field.name(), value));
    }
}
struct DiagnosticBufferLayer;
impl<S: Subscriber> Layer<S> for DiagnosticBufferLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let line = format!("{}.{:03} {:<5} {} {}", now.as_secs(), now.subsec_millis(), *metadata.level(), metadata.target(), fields.fields.join(" "));
        let mut logs = log_buffer().lock().unwrap_or_else(|p| p.into_inner());
        if logs.len() >= MAX_DIAGNOSTIC_LOGS { logs.pop_front(); }
        logs.push_back(line);
    }
}
#[uniffi::export]
pub fn recent_log_entries() -> Vec<String> {
    log_buffer().lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect()
}
#[uniffi::export]
pub fn clear_log_entries() {
    log_buffer().lock().unwrap_or_else(|p| p.into_inner()).clear();
}
#[uniffi::export]
pub fn init_logging() {
    INIT.call_once(|| {
        #[cfg(target_os = "android")]
        {
            let bridge_filter = Targets::new().with_target("rust_mobile_ffi_bridge", Level::TRACE);
            let core_filter = Targets::new().with_target("clipx_core", Level::TRACE);
            let diagnostic_filter = Targets::new().with_target("rust_mobile_ffi_bridge", Level::TRACE).with_target("clipx_core", Level::TRACE);
            tracing_subscriber::registry()
                .with(DiagnosticBufferLayer.with_filter(diagnostic_filter))
                .with(paranoid_android::layer("rust-mobile-ffi-bridge").with_ansi(false).with_filter(bridge_filter))
                .with(paranoid_android::layer("clipx-core").with_ansi(false).with_filter(core_filter))
                .init();
        }
        #[cfg(target_os = "ios")]
        { todo!("Not implemented yet") }
    });
}
