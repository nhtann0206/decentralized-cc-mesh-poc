//! TEE Circuit Breaker — prevents cascading failures when ATECC608A is unresponsive.
//!
//! States:
//! - **Closed** (normal): Signing requests go to ATECC608A
//! - **Open** (degraded): Skip ATECC608A, fallback to software hot keys
//! - **HalfOpen** (probing): Allow one request through to test recovery
//!
//! Transitions:
//! - Closed → Open: After `failure_threshold` consecutive failures
//! - Open → HalfOpen: After `recovery_timeout` elapses
//! - HalfOpen → Closed: Probe request succeeds
//! - HalfOpen → Open: Probe request fails (backoff doubles)

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit breaker state for TEE hardware operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CircuitState {
    /// Normal operation — requests go to ATECC608A.
    Closed = 0,
    /// Degraded — ATECC608A unresponsive, fallback to software signing.
    Open = 1,
    /// Probing — allow one request to test if ATECC608A recovered.
    HalfOpen = 2,
}

impl CircuitState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Closed,
            1 => Self::Open,
            2 => Self::HalfOpen,
            _ => Self::Closed,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Circuit breaker for TEE I2C/ATECC608A operations.
///
/// Thread-safe via atomics + mutex (only for timestamp).
/// Designed for the signing hot path where lock contention must be minimal.
pub struct TeeCircuitBreaker {
    state: AtomicU8,
    failure_count: AtomicU32,
    failure_threshold: u32,
    base_recovery_timeout: Duration,
    max_recovery_timeout: Duration,
    last_state_change: Mutex<Option<Instant>>,
    backoff_multiplier: AtomicU32,
}

impl TeeCircuitBreaker {
    /// Default: 3 consecutive failures to trip, 30s initial recovery, 5min max.
    pub fn new(failure_threshold: u32, base_recovery_timeout: Duration) -> Self {
        tracing::info!(
            target: "node_backend::tee",
            failure_threshold,
            recovery_timeout_ms = base_recovery_timeout.as_millis() as u64,
            "TeeCircuitBreaker initialized"
        );

        Self {
            state: AtomicU8::new(CircuitState::Closed as u8),
            failure_count: AtomicU32::new(0),
            failure_threshold,
            base_recovery_timeout,
            max_recovery_timeout: Duration::from_secs(300), // 5 minutes
            last_state_change: Mutex::new(None),
            backoff_multiplier: AtomicU32::new(1),
        }
    }

    /// Check if the circuit allows a hardware request through.
    ///
    /// - Closed → true (normal operation)
    /// - Open → check if recovery timeout elapsed → transition to HalfOpen → true
    /// - Open (timeout not elapsed) → false
    /// - HalfOpen → true (probe request)
    pub fn should_allow(&self) -> bool {
        let state = self.state();

        match state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true,
            CircuitState::Open => {
                // Check if enough time has passed for a probe
                let current_timeout = self.current_recovery_timeout();
                let should_probe = {
                    let last = self.last_state_change.lock().unwrap();
                    last.map_or(true, |t| t.elapsed() >= current_timeout)
                };

                if should_probe {
                    // Transition to HalfOpen for probe
                    self.transition_to(CircuitState::HalfOpen);
                    tracing::info!(
                        target: "node_backend::tee",
                        backoff_multiplier = self.backoff_multiplier.load(Ordering::Relaxed),
                        "Circuit breaker → HalfOpen (probing ATECC608A)"
                    );
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a successful hardware operation.
    ///
    /// - Closed → reset failure count
    /// - HalfOpen → transition to Closed (ATECC608A recovered)
    pub fn record_success(&self) {
        let prev_state = self.state();
        self.failure_count.store(0, Ordering::Relaxed);

        if prev_state == CircuitState::HalfOpen {
            self.backoff_multiplier.store(1, Ordering::Relaxed);
            self.transition_to(CircuitState::Closed);
            tracing::info!(
                target: "node_backend::tee",
                "Circuit breaker → Closed (ATECC608A recovered)"
            );
        }
    }

    /// Record a failed hardware operation.
    ///
    /// - Closed → increment failure count, trip to Open if threshold reached
    /// - HalfOpen → reopen with increased backoff
    pub fn record_failure(&self) {
        let state = self.state();

        match state {
            CircuitState::Closed => {
                let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= self.failure_threshold {
                    self.transition_to(CircuitState::Open);
                    tracing::error!(
                        target: "node_backend::tee",
                        consecutive_failures = count,
                        threshold = self.failure_threshold,
                        "Circuit breaker → Open (ATECC608A unresponsive)"
                    );
                } else {
                    tracing::warn!(
                        target: "node_backend::tee",
                        consecutive_failures = count,
                        threshold = self.failure_threshold,
                        "ATECC608A failure recorded ({}/{})",
                        count,
                        self.failure_threshold
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — reopen with exponential backoff
                let _prev = self.backoff_multiplier.fetch_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |m| {
                        let next = m.saturating_mul(2);
                        let max_mult = (self.max_recovery_timeout.as_millis()
                            / self.base_recovery_timeout.as_millis().max(1))
                            as u32;
                        Some(next.min(max_mult.max(1)))
                    },
                );
                let new_mult = self.backoff_multiplier.load(Ordering::Relaxed);
                self.transition_to(CircuitState::Open);
                tracing::warn!(
                    target: "node_backend::tee",
                    backoff_multiplier = new_mult,
                    next_probe_ms = self.current_recovery_timeout().as_millis() as u64,
                    "Circuit breaker → Open (probe failed, backoff increased)"
                );
            }
            CircuitState::Open => {
                // Already open — nothing to do
            }
        }
    }

    /// Get current circuit state.
    pub fn state(&self) -> CircuitState {
        CircuitState::from_u8(self.state.load(Ordering::Relaxed))
    }

    /// Get consecutive failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::Relaxed)
    }

    /// Get current recovery timeout (with exponential backoff).
    pub fn current_recovery_timeout(&self) -> Duration {
        let mult = self.backoff_multiplier.load(Ordering::Relaxed).max(1);
        let timeout = self.base_recovery_timeout.saturating_mul(mult);
        if timeout > self.max_recovery_timeout {
            self.max_recovery_timeout
        } else {
            timeout
        }
    }

    fn transition_to(&self, new_state: CircuitState) {
        self.state.store(new_state as u8, Ordering::Relaxed);
        let mut last = self.last_state_change.lock().unwrap();
        *last = Some(Instant::now());
    }
}

impl std::fmt::Debug for TeeCircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeeCircuitBreaker")
            .field("state", &self.state())
            .field("failure_count", &self.failure_count())
            .field("failure_threshold", &self.failure_threshold)
            .field("recovery_timeout", &self.current_recovery_timeout())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaker(threshold: u32, timeout_ms: u64) -> TeeCircuitBreaker {
        TeeCircuitBreaker::new(threshold, Duration::from_millis(timeout_ms))
    }

    #[test]
    fn test_success_resets_failure_count() {
        let cb = breaker(3, 100);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_trips_to_open_after_threshold() {
        let cb = breaker(3, 100);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure(); // 3rd failure
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.should_allow()); // timeout not elapsed
    }

    #[test]
    fn test_open_blocks_requests_until_timeout() {
        let cb = breaker(1, 50); // 50ms timeout
        cb.record_failure(); // trip immediately
        assert_eq!(cb.state(), CircuitState::Open);

        // Should block immediately
        assert!(!cb.should_allow());

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(60));

        // Now should transition to HalfOpen and allow
        assert!(cb.should_allow());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[test]
    fn test_half_open_success_closes_circuit() {
        let cb = breaker(1, 10);
        cb.record_failure(); // Open
        std::thread::sleep(Duration::from_millis(15));

        cb.should_allow(); // → HalfOpen
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success(); // → Closed
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_half_open_failure_reopens_with_backoff() {
        let cb = breaker(1, 10);
        cb.record_failure(); // Open
        std::thread::sleep(Duration::from_millis(15));

        cb.should_allow(); // → HalfOpen
        cb.record_failure(); // probe failed → Open again

        assert_eq!(cb.state(), CircuitState::Open);
        // Backoff should have increased
        assert!(cb.current_recovery_timeout() > Duration::from_millis(10));
    }

    #[test]
    fn test_exponential_backoff_caps_at_max() {
        let cb = TeeCircuitBreaker {
            state: AtomicU8::new(CircuitState::Closed as u8),
            failure_count: AtomicU32::new(0),
            failure_threshold: 1,
            base_recovery_timeout: Duration::from_secs(30),
            max_recovery_timeout: Duration::from_secs(300),
            last_state_change: Mutex::new(None),
            backoff_multiplier: AtomicU32::new(1),
        };

        // Simulate repeated probe failures
        cb.backoff_multiplier.store(16, Ordering::Relaxed);
        let timeout = cb.current_recovery_timeout();
        // 30s * 16 = 480s > 300s max → should be capped at 300s
        assert_eq!(timeout, Duration::from_secs(300));
    }

    #[test]
    fn test_backoff_resets_on_recovery() {
        let cb = breaker(1, 10);
        cb.record_failure(); // Open
        cb.backoff_multiplier.store(4, Ordering::Relaxed);

        std::thread::sleep(Duration::from_millis(50));
        cb.should_allow(); // → HalfOpen
        cb.record_success(); // → Closed, backoff resets

        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.current_recovery_timeout(), Duration::from_millis(10));
    }

    #[test]
    fn test_concurrent_failures() {
        use std::sync::Arc;
        let cb = Arc::new(breaker(10, 100));
        let mut handles = vec![];

        for _ in 0..10 {
            let cb_clone = cb.clone();
            handles.push(std::thread::spawn(move || {
                cb_clone.record_failure();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // After 10 concurrent failures with threshold 10, should be Open
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_backoff_multiplier_doubles_exactly_once_per_probe_failure() {
        let cb = TeeCircuitBreaker::new(1, Duration::from_secs(30));
        cb.record_failure(); // → Open

        // Force HalfOpen by backdating last_state_change
        {
            let mut last = cb.last_state_change.lock().unwrap();
            *last = Some(Instant::now() - Duration::from_secs(60));
        }
        assert!(cb.should_allow()); // → HalfOpen
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_failure(); // Probe failed → Open with backoff

        // Multiplier should be exactly 2 (initial 1 × 2), not 4 (double-doubled)
        let stored = cb.backoff_multiplier.load(Ordering::Relaxed);
        assert_eq!(stored, 2, "Backoff multiplier should double exactly once per probe failure");

        // Recovery timeout should reflect the stored multiplier: 30s × 2 = 60s
        assert_eq!(cb.current_recovery_timeout(), Duration::from_secs(60));
    }
}
