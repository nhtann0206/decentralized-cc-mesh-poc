//! SLM Observability Metrics — Prometheus counters, histograms, and gauges.
//!
//! Provides structured metrics for SLM operations:
//! - Inference duration and routing target distribution
//! - Memory usage tracking
//! - Model loading duration and lifecycle events
//! - Routing decision counts
//!
//! All metrics use the `slm_` prefix for Prometheus namespace consistency.

use lazy_static::lazy_static;
use prometheus::{
    register_histogram, register_int_gauge,
    Histogram, HistogramOpts, IntCounterVec, IntGauge, Opts,
};

lazy_static! {
    // ── Inference Metrics ────────────────────────────────────────

    /// Duration of inference operations (seconds).
    pub static ref SLM_INFERENCE_DURATION: Histogram = register_histogram!(
        HistogramOpts::new(
            "slm_inference_duration_seconds",
            "Duration of SLM inference operations"
        ).buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0])
    ).unwrap();

    /// Total inference operations.
    /// Labels: target=local|cloud|peer, outcome=success|error
    pub static ref SLM_INFERENCE_TOTAL: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("slm_inference_total", "Total SLM inference operations"),
        &["target", "outcome"]
    ).unwrap();

    // ── Routing Metrics ──────────────────────────────────────────

    /// Routing decisions by target.
    /// Labels: target=local|cloud|peer, reason=available|fallback|preference
    pub static ref SLM_ROUTING_DECISIONS: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("slm_routing_decisions_total", "Routing decision counts by target"),
        &["target", "reason"]
    ).unwrap();

    // ── Memory Metrics ───────────────────────────────────────────

    /// Current memory used by loaded models (MB).
    pub static ref SLM_MEMORY_USED_MB: IntGauge = register_int_gauge!(
        "slm_memory_used_mb",
        "Current memory used by loaded SLM models in MB"
    ).unwrap();

    /// Configured memory limit (MB).
    pub static ref SLM_MEMORY_LIMIT_MB: IntGauge = register_int_gauge!(
        "slm_memory_limit_mb",
        "Configured memory limit for SLM models in MB"
    ).unwrap();

    // ── Lifecycle Metrics ────────────────────────────────────────

    /// Duration of model loading operations (seconds).
    pub static ref SLM_MODEL_LOAD_DURATION: Histogram = register_histogram!(
        HistogramOpts::new(
            "slm_model_load_duration_seconds",
            "Duration of model loading operations"
        ).buckets(vec![1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0])
    ).unwrap();

    /// Total model lifecycle events.
    /// Labels: event=load|unload|swap|pull, outcome=success|error
    pub static ref SLM_LIFECYCLE_TOTAL: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("slm_lifecycle_events_total", "Model lifecycle event counts"),
        &["event", "outcome"]
    ).unwrap();

    /// Number of models currently loaded.
    pub static ref SLM_MODELS_LOADED: IntGauge = register_int_gauge!(
        "slm_models_loaded",
        "Number of models currently loaded in RAM"
    ).unwrap();

    // ── Provider Metrics ─────────────────────────────────────────

    /// Whether Ollama is currently reachable (1=yes, 0=no).
    pub static ref SLM_OLLAMA_AVAILABLE: IntGauge = register_int_gauge!(
        "slm_ollama_available",
        "Whether the Ollama backend is reachable (1=yes, 0=no)"
    ).unwrap();
}

/// Record an inference result.
pub fn record_inference(target: &str, outcome: &str, duration_secs: f64) {
    SLM_INFERENCE_TOTAL.with_label_values(&[target, outcome]).inc();
    SLM_INFERENCE_DURATION.observe(duration_secs);
}

/// Record a routing decision.
pub fn record_routing_decision(target: &str, reason: &str) {
    SLM_ROUTING_DECISIONS.with_label_values(&[target, reason]).inc();
}

/// Record a lifecycle event.
pub fn record_lifecycle_event(event: &str, outcome: &str) {
    SLM_LIFECYCLE_TOTAL.with_label_values(&[event, outcome]).inc();
}

/// Update memory gauge values.
pub fn update_memory_gauges(used_mb: u64, limit_mb: u64) {
    SLM_MEMORY_USED_MB.set(used_mb as i64);
    SLM_MEMORY_LIMIT_MB.set(limit_mb as i64);
}

