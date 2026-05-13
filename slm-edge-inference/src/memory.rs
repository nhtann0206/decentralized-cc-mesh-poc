//! SLM Memory Budget Manager — tracks and enforces RAM limits for model loading.
//!
//! On edge devices (Radxa Zero 3W, 1GB RAM), memory is the primary constraint.
//! This module:
//! - Reads system available memory
//! - Tracks loaded model memory usage
//! - Enforces hard limits before model loading
//! - Provides budget status for status endpoint + routing decisions

use crate::slm::config::SlmConfig;
use crate::slm::models::MemoryBudget;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

/// Manages memory budget for SLM model loading.
///
/// Thread-safe via atomics — can be shared across async tasks.
pub struct MemoryBudgetManager {
    /// Configured memory limit in MB.
    limit_mb: u64,
    /// Current memory used by loaded models in MB.
    used_mb: AtomicU64,
}

impl MemoryBudgetManager {
    pub fn new(config: &SlmConfig) -> Self {
        Self {
            limit_mb: config.memory_limit_mb,
            used_mb: AtomicU64::new(0),
        }
    }

    /// Check if a model of the given size can be loaded within budget.
    pub fn can_load(&self, required_mb: u64) -> bool {
        let used = self.used_mb.load(Ordering::Relaxed);
        used + required_mb <= self.limit_mb
    }

    /// Reserve memory for a model about to be loaded.
    /// Returns false if budget would be exceeded.
    ///
    /// Uses CAS loop to prevent race conditions where two concurrent
    /// reserve() calls could both succeed and exceed the budget.
    pub fn reserve(&self, model_mb: u64) -> bool {
        loop {
            let used = self.used_mb.load(Ordering::Acquire);
            if used + model_mb > self.limit_mb {
                warn!(
                    target: "node_backend::slm",
                    required_mb = model_mb,
                    used_mb = used,
                    limit_mb = self.limit_mb,
                    "Model loading rejected: exceeds memory budget"
                );
                return false;
            }
            match self.used_mb.compare_exchange(
                used,
                used + model_mb,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => continue, // Another thread changed used_mb, retry
            }
        }
    }

    /// Release memory after a model is unloaded.
    ///
    /// Uses CAS loop to prevent TOCTOU race between reading and subtracting.
    /// Saturates to 0 if releasing more than currently tracked (defensive).
    pub fn release(&self, model_mb: u64) {
        loop {
            let used = self.used_mb.load(Ordering::Acquire);
            let actual_release = model_mb.min(used);
            let new_used = used - actual_release;
            match self.used_mb.compare_exchange(
                used,
                new_used,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if actual_release < model_mb {
                        warn!(
                            target: "node_backend::slm",
                            requested_mb = model_mb,
                            actual_mb = actual_release,
                            "Released less memory than requested — possible tracking error"
                        );
                    }
                    tracing::debug!(
                        target: "node_backend::slm",
                        released_mb = actual_release,
                        previous_used_mb = used,
                        "Memory released after model unload"
                    );
                    return;
                }
                Err(_) => continue,
            }
        }
    }

    /// Reset tracked memory to zero (e.g., after Ollama restart).
    pub fn reset(&self) {
        self.used_mb.store(0, Ordering::Relaxed);
    }

    /// Get current memory budget status.
    pub fn budget(&self) -> MemoryBudget {
        let used = self.used_mb.load(Ordering::Relaxed);
        MemoryBudget {
            used_mb: used,
            limit_mb: self.limit_mb,
            available_mb: self.limit_mb.saturating_sub(used),
            loaded_models: vec![], // Populated by lifecycle manager
        }
    }

    /// Get available MB.
    pub fn available_mb(&self) -> u64 {
        self.limit_mb.saturating_sub(self.used_mb.load(Ordering::Relaxed))
    }

    /// Get the configured limit.
    pub fn limit_mb(&self) -> u64 {
        self.limit_mb
    }

    /// Get current usage.
    pub fn used_mb(&self) -> u64 {
        self.used_mb.load(Ordering::Relaxed)
    }

    /// Read system available memory from /proc/meminfo (Linux) or sysinfo (macOS).
    ///
    /// Informational only — used by status endpoint. Memory budget enforcement
    /// uses the configured limit (SLM_MEMORY_LIMIT_MB), not this value.
    /// Returns available memory in MB, or None if unavailable.
    pub fn system_available_memory_mb() -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
                for line in contents.lines() {
                    if line.starts_with("MemAvailable:") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            if let Ok(kb) = parts[1].parse::<u64>() {
                                return Some(kb / 1024);
                            }
                        }
                    }
                }
            }
            None
        }
        #[cfg(target_os = "macos")]
        {
            use sysinfo::System;
            let mut sys = System::new_all();
            sys.refresh_memory();
            Some(sys.available_memory() / (1024 * 1024))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(limit_mb: u64) -> SlmConfig {
        SlmConfig {
            enabled: true,
            memory_limit_mb: limit_mb,
            ..SlmConfig::default()
        }
    }

    #[test]
    fn test_can_load_within_budget() {
        let mgr = MemoryBudgetManager::new(&test_config(512));
        assert!(mgr.can_load(300));
        assert!(mgr.can_load(512));
        assert!(!mgr.can_load(513));
    }

    #[test]
    fn test_reserve_and_release() {
        let mgr = MemoryBudgetManager::new(&test_config(512));
        assert!(mgr.reserve(300));
        assert_eq!(mgr.used_mb(), 300);
        assert_eq!(mgr.available_mb(), 212);

        // Can't reserve another 300
        assert!(!mgr.reserve(300));

        // Release and try again
        mgr.release(300);
        assert_eq!(mgr.used_mb(), 0);
        assert!(mgr.reserve(300));
    }

    #[test]
    fn test_reserve_exact_limit() {
        let mgr = MemoryBudgetManager::new(&test_config(512));
        assert!(mgr.reserve(512));
        assert!(!mgr.reserve(1));
    }

    #[test]
    fn test_release_more_than_used_saturates() {
        let mgr = MemoryBudgetManager::new(&test_config(512));
        mgr.reserve(100);
        mgr.release(200); // Release more than used
        // CAS loop ensures exact saturation to 0 — no race, no underflow
        assert_eq!(mgr.used_mb(), 0);
    }

    #[test]
    fn test_concurrent_reserve_respects_budget() {
        use std::sync::Arc;
        let mgr = Arc::new(MemoryBudgetManager::new(&test_config(512)));
        let mut handles = vec![];

        // 10 threads each try to reserve 100MB — only 5 should succeed (5*100=500 <= 512)
        for _ in 0..10 {
            let mgr_clone = mgr.clone();
            handles.push(std::thread::spawn(move || {
                mgr_clone.reserve(100)
            }));
        }

        let successes: usize = handles.into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&ok| ok)
            .count();

        assert_eq!(successes, 5, "Exactly 5 reserves should succeed (500 <= 512)");
        assert_eq!(mgr.used_mb(), 500);
    }

}
