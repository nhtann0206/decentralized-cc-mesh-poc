//! TEE Observability Metrics — Prometheus counters, histograms, and gauges.
//!
//! Provides structured metrics for TEE operations:
//! - Signing latency and strategy distribution (hot/cold/fallback)
//! - I2C bus contention and timeout tracking
//! - Watchdog heartbeat regularity
//! - Attestation success rates
//!
//! All metrics use the `tee_` prefix for Prometheus namespace consistency.

use lazy_static::lazy_static;
use prometheus::{
    register_histogram, register_int_gauge,
    Histogram, HistogramOpts, IntCounterVec, IntGauge, Opts,
};

lazy_static! {
    // ── Signing Metrics ──────────────────────────────────────────

    /// Duration of signing operations (seconds).
    /// Labels: strategy=hot|cold|fallback
    pub static ref TEE_SIGNING_DURATION: Histogram = register_histogram!(
        HistogramOpts::new(
            "tee_signing_duration_seconds",
            "Duration of TEE signing operations"
        ).buckets(vec![0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0])
    ).unwrap();

    /// Total signing operations.
    /// Labels: strategy=hot|cold|fallback, outcome=success|error
    pub static ref TEE_SIGNING_TOTAL: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_signing_total", "Total TEE signing operations"),
        &["strategy", "outcome"]
    ).unwrap();

    /// Signing strategy selection count.
    /// Labels: strategy=hot|cold
    pub static ref TEE_SIGNING_STRATEGY: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_signing_strategy_selection", "Signing strategy selections by BackendTeeSignerFactory"),
        &["strategy"]
    ).unwrap();

    // ── I2C Metrics ──────────────────────────────────────────────

    /// I2C request duration (seconds).
    pub static ref TEE_I2C_DURATION: Histogram = register_histogram!(
        HistogramOpts::new(
            "tee_i2c_request_duration_seconds",
            "Duration of I2C bus operations"
        ).buckets(vec![0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500])
    ).unwrap();

    /// I2C timeouts (500ms threshold).
    pub static ref TEE_I2C_TIMEOUT_TOTAL: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_i2c_timeout_total", "I2C bus operation timeouts"),
        &["operation"]
    ).unwrap();

    /// I2C mutex contention warnings (wait > 10ms, NFR-004).
    pub static ref TEE_I2C_CONTENTION: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_i2c_contention_warnings", "I2C mutex contention events (wait > 10ms)"),
        &["operation"]
    ).unwrap();

    // ── Watchdog Metrics ─────────────────────────────────────────

    /// Actual interval between watchdog heartbeats (seconds).
    pub static ref TEE_WATCHDOG_HEARTBEAT_INTERVAL: Histogram = register_histogram!(
        HistogramOpts::new(
            "tee_watchdog_heartbeat_interval_seconds",
            "Actual interval between watchdog heartbeats"
        ).buckets(vec![0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0])
    ).unwrap();

    /// Current consecutive miss count.
    pub static ref TEE_WATCHDOG_CONSECUTIVE_MISSES: IntGauge = register_int_gauge!(
        "tee_watchdog_consecutive_misses",
        "Current number of consecutive watchdog misses"
    ).unwrap();

    // ── Attestation Metrics ──────────────────────────────────────

    /// Total attestation requests.
    /// Labels: outcome=success|error
    pub static ref TEE_ATTESTATION_TOTAL: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_attestation_requests_total", "Total attestation challenge-response requests"),
        &["outcome"]
    ).unwrap();

    /// Attestation duration (seconds).
    pub static ref TEE_ATTESTATION_DURATION: Histogram = register_histogram!(
        HistogramOpts::new(
            "tee_attestation_duration_seconds",
            "Duration of attestation operations"
        ).buckets(vec![0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.0])
    ).unwrap();

    /// Number of registered trust anchors.
    pub static ref TEE_ATTESTATION_REGISTRY_SIZE: IntGauge = register_int_gauge!(
        "tee_attestation_registry_size",
        "Number of registered trust anchors in attestation registry"
    ).unwrap();

    // ── Circuit Breaker Metrics ──────────────────────────────────

    /// Circuit breaker state transitions.
    /// Labels: from=closed|open|half_open, to=closed|open|half_open
    pub static ref TEE_CIRCUIT_BREAKER_TRANSITIONS: IntCounterVec = prometheus::register_int_counter_vec!(
        Opts::new("tee_circuit_breaker_transitions_total", "Circuit breaker state transitions"),
        &["from", "to"]
    ).unwrap();
}

/// Record a signing strategy selection (called by BackendTeeSignerFactory).
pub fn record_strategy_selection(strategy: &str) {
    TEE_SIGNING_STRATEGY.with_label_values(&[strategy]).inc();
}

/// Record a signing operation result.
pub fn record_signing(strategy: &str, outcome: &str, duration_secs: f64) {
    TEE_SIGNING_TOTAL.with_label_values(&[strategy, outcome]).inc();
    TEE_SIGNING_DURATION.observe(duration_secs);
}

/// Record a circuit breaker state transition.
pub fn record_circuit_breaker_transition(from: &str, to: &str) {
    TEE_CIRCUIT_BREAKER_TRANSITIONS.with_label_values(&[from, to]).inc();
}

