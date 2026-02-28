use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::ai::errors::AiErrorKind;

fn ai_perf_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("OPENMANGO_PERF_AI").is_some())
}

static TOTAL_REQUESTS: AtomicU64 = AtomicU64::new(0);
static FAILED_REQUESTS: AtomicU64 = AtomicU64::new(0);
static SUCCESS_REQUESTS: AtomicU64 = AtomicU64::new(0);
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub struct AiRequestSpan {
    id: u64,
    provider: String,
    model: String,
    session: String,
    started_at: Instant,
}

impl AiRequestSpan {
    pub fn start(
        provider: impl Into<String>,
        model: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
        let span = Self {
            id: REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            provider: provider.into(),
            model: model.into(),
            session: session.into(),
            started_at: Instant::now(),
        };
        if ai_perf_enabled() {
            log::info!(
                "[perf-ai] request.start id={} provider={} model={} session={}",
                span.id,
                span.provider,
                span.model,
                span.session
            );
        }
        span
    }

    pub fn finish_ok(self, chars: usize) {
        SUCCESS_REQUESTS.fetch_add(1, Ordering::Relaxed);
        if ai_perf_enabled() {
            let elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
            log::info!(
                "[perf-ai] request.ok id={} provider={} model={} ms={elapsed_ms:.2} chars={} totals={}/{}/{}",
                self.id,
                self.provider,
                self.model,
                chars,
                TOTAL_REQUESTS.load(Ordering::Relaxed),
                SUCCESS_REQUESTS.load(Ordering::Relaxed),
                FAILED_REQUESTS.load(Ordering::Relaxed)
            );
        }
    }

    pub fn finish_err(self, kind: AiErrorKind) {
        FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed);
        if ai_perf_enabled() {
            let elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
            log::info!(
                "[perf-ai] request.err id={} provider={} model={} ms={elapsed_ms:.2} kind={kind:?} totals={}/{}/{}",
                self.id,
                self.provider,
                self.model,
                TOTAL_REQUESTS.load(Ordering::Relaxed),
                SUCCESS_REQUESTS.load(Ordering::Relaxed),
                FAILED_REQUESTS.load(Ordering::Relaxed)
            );
        }
    }
}
