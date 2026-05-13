//! LDK Signer Adapter — bridges `VlsProvider` into LDK signing operations.
//!
//! # Architecture
//!
//! LDK uses associated types (`type EcdsaSigner`) in `SignerProvider`, making
//! `dyn SignerProvider` (dynamic dispatch) impossible at compile time.
//!
//! The `ldk-node` fork (`LDK fork @ 039-tee-hybrid-signer`) solves this
//! with a `HybridSigner` enum for **runtime dispatch**:
//!
//! ```rust,ignore
//! pub enum HybridSigner {
//!     Software(InMemorySigner),  // Hot path: RAM keys, < 5ms
//!     Tee(InMemorySigner),       // Cold path: Phase 2 → Atecc608aSigner
//! }
//! ```
//!
//! # Integration with Main's Builtin-App Architecture
//!
//! In main's architecture, LDK is owned by the `builtin-apps/ldk-node` crate (cdylib).
//! The backend does NOT depend on `ldk-node` directly. Therefore:
//!
//! - `VlsSignerProvider` lives here (backend TEE module) — signing logic + circuit breaker
//! - `BackendTeeSignerFactory` (implements `TeeSignerFactory` from fork) lives in the
//!   builtin ldk-node app, which has direct access to `ldk_node::Builder`
//! - The builtin app reads TEE config via `get_config("tee_enabled")` and injects the
//!   factory into `NodeBuilder::set_tee_signer_factory()` during LDK node build
//! - For hardware signing delegation, the builtin app calls back to the backend's
//!   TEE service via CapabilityRouter (`core.tee.sign_commitment`, etc.)
//!
//! This module also provides `SignerStrategy` and `derive_signer_strategy()` as
//! shared logic that both the backend and the builtin app can use.

use std::sync::Arc;

use super::circuit_breaker::TeeCircuitBreaker;
use super::error::TeeError;
use super::traits::{SigningStrategy, VlsProvider};

/// Signer strategy for a Lightning channel.
///
/// Maps to `ldk_node::HybridSigner` from the custom fork.
/// Used by both the backend (for logic/tests) and the builtin app (for actual LDK injection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerStrategy {
    /// Hot path: RAM-only keys, < 5ms latency. For small HTLCs below threshold.
    Software,
    /// Cold path: Hardware-backed signing via ATECC608A. For commitments and high-value HTLCs.
    Tee,
}

/// Determine signer strategy for a channel based on value vs threshold.
///
/// Shared logic used by both `BackendTeeSignerFactory` (in builtin app) and
/// `VlsSignerProvider` (in backend).
///
/// - `channel_value_satoshis * 1000 < threshold_msat` → `Software`
/// - Otherwise → `Tee`
pub fn derive_signer_strategy(channel_value_satoshis: u64, threshold_msat: u64) -> SignerStrategy {
    let channel_value_msat = channel_value_satoshis.saturating_mul(1000);
    if channel_value_msat < threshold_msat {
        SignerStrategy::Software
    } else {
        SignerStrategy::Tee
    }
}

/// Adapter that bridges `VlsProvider` into LDK's signing flow.
///
/// # Signing Strategy
///
/// - **Hot path** (RAM keys): HTLC forwards < threshold (default 500k sats)
///   - Latency: < 5ms (NFR-003)
///   - Keys: Software ECDSA, zeroize-protected via `SecureMemory`
/// - **Cold path** (ATECC608A): Commitment txs, channel closes, large HTLCs
///   - Latency: 25-100ms (NFR-002)
///   - Keys: Hardware-isolated, never exported
pub struct VlsSignerProvider {
    vls: Arc<dyn VlsProvider>,
    htlc_threshold_msat: u64,
    circuit_breaker: Option<Arc<TeeCircuitBreaker>>,
}

impl VlsSignerProvider {
    pub fn new(vls: Arc<dyn VlsProvider>, htlc_threshold_msat: u64) -> Self {
        tracing::info!(
            target: "node_backend::tee",
            is_hardware_backed = vls.is_hardware_backed(),
            htlc_threshold_msat,
            "VlsSignerProvider initialized"
        );

        Self {
            vls,
            htlc_threshold_msat,
            circuit_breaker: None,
        }
    }

    /// Attach a circuit breaker for fail-closed behavior on high-value channels.
    pub fn with_circuit_breaker(mut self, cb: Arc<TeeCircuitBreaker>) -> Self {
        self.circuit_breaker = Some(cb);
        self
    }

    /// Sign a commitment transaction via the VLS provider.
    /// Always uses cold path (ATECC608A in production).
    /// Fail-closed: if circuit breaker is Open, returns Err.
    pub async fn sign_commitment(&self, tx_hash: &[u8; 32]) -> Result<Vec<u8>, TeeError> {
        if let Some(cb) = &self.circuit_breaker {
            if !cb.should_allow() {
                tracing::error!(
                    target: "node_backend::tee",
                    "Refusing commitment signing — circuit breaker Open (fail-closed)"
                );
                return Err(TeeError::SigningError(
                    "Hardware signer unavailable (circuit breaker open) — refusing commitment signing".to_string(),
                ));
            }
        }

        self.vls.sign_commitment(tx_hash).await
    }

    /// Sign an HTLC transaction via the VLS provider.
    /// Hot/cold path based on amount vs threshold.
    /// Fail-closed for high-value HTLCs when circuit breaker is Open.
    pub async fn sign_htlc(
        &self,
        htlc_hash: &[u8; 32],
        amount_msat: u64,
    ) -> Result<(Vec<u8>, SigningStrategy), TeeError> {
        if amount_msat >= self.htlc_threshold_msat {
            if let Some(cb) = &self.circuit_breaker {
                if !cb.should_allow() {
                    tracing::error!(
                        target: "node_backend::tee",
                        amount_msat,
                        threshold_msat = self.htlc_threshold_msat,
                        "Refusing high-value HTLC signing — circuit breaker Open (fail-closed)"
                    );
                    return Err(TeeError::SigningError(
                        "Hardware signer unavailable (circuit breaker open) — refusing high-value HTLC signing".to_string(),
                    ));
                }
            }
        }

        self.vls.sign_htlc(htlc_hash, amount_msat).await
    }

    /// Sign arbitrary data via the VLS provider.
    pub async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, TeeError> {
        self.vls.sign(data).await
    }

    pub fn is_hardware_backed(&self) -> bool {
        self.vls.is_hardware_backed()
    }
}

impl std::fmt::Debug for VlsSignerProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VlsSignerProvider")
            .field("hardware_backed", &self.vls.is_hardware_backed())
            .field("htlc_threshold_msat", &self.htlc_threshold_msat)
            .field("circuit_breaker", &self.circuit_breaker.as_ref().map(|cb| cb.state()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tee::config::DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT;
    use crate::tee::providers::MockVlsProvider;
    use std::time::Duration;

    #[tokio::test]
    async fn test_adapter_sign_htlc_hot_path() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT);

        let (sig, strategy) = adapter.sign_htlc(&[0x01; 32], 50_000_000).await.unwrap();
        assert_eq!(sig.len(), 64);
        assert!(matches!(strategy, SigningStrategy::HotPath));
    }

    #[tokio::test]
    async fn test_adapter_sign_htlc_cold_path() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT);

        let (sig, strategy) = adapter.sign_htlc(&[0x01; 32], 600_000_000).await.unwrap();
        assert_eq!(sig.len(), 64);
        assert!(matches!(strategy, SigningStrategy::ColdPath));
    }

    #[tokio::test]
    async fn test_fail_closed_commitment_when_cb_open() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let cb = Arc::new(TeeCircuitBreaker::new(1, Duration::from_secs(60)));
        cb.record_failure();
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT)
            .with_circuit_breaker(cb);

        let result = adapter.sign_commitment(&[0x01; 32]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("circuit breaker"));
    }

    #[tokio::test]
    async fn test_fail_closed_high_value_htlc_when_cb_open() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let cb = Arc::new(TeeCircuitBreaker::new(1, Duration::from_secs(60)));
        cb.record_failure();
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT)
            .with_circuit_breaker(cb);

        let result = adapter.sign_htlc(&[0x01; 32], 600_000_000).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fail_open_low_value_htlc_when_cb_open() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let cb = Arc::new(TeeCircuitBreaker::new(1, Duration::from_secs(60)));
        cb.record_failure();
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT)
            .with_circuit_breaker(cb);

        let result = adapter.sign_htlc(&[0x01; 32], 50_000_000).await;
        assert!(result.is_ok());
        let (_, strategy) = result.unwrap();
        assert!(matches!(strategy, SigningStrategy::HotPath));
    }

    #[tokio::test]
    async fn test_no_cb_never_fails_closed() {
        let mock_vls = Arc::new(MockVlsProvider::default());
        let adapter = VlsSignerProvider::new(mock_vls, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT);

        assert!(adapter.sign_commitment(&[0x01; 32]).await.is_ok());
        assert!(adapter.sign_htlc(&[0x01; 32], 600_000_000).await.is_ok());
    }

    #[test]
    fn test_derive_strategy_below_threshold() {
        assert_eq!(
            derive_signer_strategy(250_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT),
            SignerStrategy::Software
        );
    }

    #[test]
    fn test_derive_strategy_above_threshold() {
        assert_eq!(
            derive_signer_strategy(600_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT),
            SignerStrategy::Tee
        );
    }

    #[test]
    fn test_derive_strategy_at_threshold() {
        assert_eq!(
            derive_signer_strategy(500_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT),
            SignerStrategy::Tee
        );
    }

    #[test]
    fn test_derive_strategy_zero_threshold_all_tee() {
        assert_eq!(derive_signer_strategy(1, 0), SignerStrategy::Tee);
    }

    #[test]
    fn test_derive_strategy_max_threshold_all_software() {
        assert_eq!(derive_signer_strategy(1_000_000, u64::MAX), SignerStrategy::Software);
    }
}
