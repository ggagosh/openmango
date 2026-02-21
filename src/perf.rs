use std::sync::OnceLock;
use std::time::Instant;

fn tabs_perf_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("OPENMANGO_PERF_TABS").is_some())
}

pub fn log_tabs_duration(label: &str, start: Instant, details: impl FnOnce() -> String) {
    if !tabs_perf_enabled() {
        return;
    }

    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    log::info!("[perf-tabs] {label} ms={elapsed_ms:.3} {}", details());
}
