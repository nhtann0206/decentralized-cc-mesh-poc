use async_trait::async_trait;
use zeroize::Zeroize;

use crate::tee::config::DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT;
use crate::tee::error::TeeError;
use crate::tee::secure_memory::SecureMemory;
use crate::tee::traits::{SigningStrategy, VlsProvider};

/// Simulated key seed for mlock protection testing.
/// In production, this would be derived from the actual entropy seed.
#[derive(Clone, Zeroize)]
struct SimulatedKeySeed([u8; 32]);

/// Mock VLS provider for development and CI/CD.
///
/// Uses software signing (no hardware). Returns `is_hardware_backed: false`.
/// Simulates the hybrid hot/cold path selection logic.
///
/// The `_key_seed` field exercises `SecureMemory` (mlock/zeroize) at runtime,
/// ensuring the memory protection lifecycle is tested during normal app boot.
pub struct MockVlsProvider {
    htlc_hot_path_threshold_msat: u64,
    /// Simulated key seed protected by mlock. Demonstrates that hot keys
    /// are pinned in RAM and zeroized on drop, even in mock mode.
    _key_seed: SecureMemory<SimulatedKeySeed>,
}

impl MockVlsProvider {
    pub fn new(htlc_hot_path_threshold_msat: u64) -> Self {
        let seed = SecureMemory::new(SimulatedKeySeed([0x42; 32]));
        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            mlock_active = seed.is_locked(),
            "MockVlsProvider initialized with SecureMemory key seed"
        );
        Self {
            htlc_hot_path_threshold_msat,
            _key_seed: seed,
        }
    }
}

impl Default for MockVlsProvider {
    fn default() -> Self {
        Self::new(DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT)
    }
}

#[async_trait]
impl VlsProvider for MockVlsProvider {
    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, TeeError> {
        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            data_len = data.len(),
            "Mock signing data"
        );

        // Return a fake 64-byte signature
        Ok(vec![0x42; 64])
    }

    async fn sign_commitment(&self, tx_hash: &[u8; 32]) -> Result<Vec<u8>, TeeError> {
        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            strategy = "cold_path",
            "Mock signing commitment transaction (always cold path)"
        );

        // Commitment transactions always use cold path (ATECC608A in production)
        let _ = tx_hash;
        Ok(vec![0x42; 64])
    }

    async fn sign_htlc(
        &self,
        htlc_hash: &[u8; 32],
        amount_msat: u64,
    ) -> Result<(Vec<u8>, SigningStrategy), TeeError> {
        let strategy = if amount_msat < self.htlc_hot_path_threshold_msat {
            SigningStrategy::HotPath
        } else {
            SigningStrategy::ColdPath
        };

        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            amount_msat = amount_msat,
            strategy = ?strategy,
            "Mock signing HTLC"
        );

        let _ = htlc_hash;
        Ok((vec![0x42; 64], strategy))
    }

    fn is_hardware_backed(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_sign() {
        let vls = MockVlsProvider::default();
        let sig = vls.sign(b"test data").await.unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[tokio::test]
    async fn test_mock_sign_commitment_always_cold() {
        let vls = MockVlsProvider::default();
        let sig = vls.sign_commitment(&[0x01; 32]).await.unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[tokio::test]
    async fn test_mock_htlc_hot_path() {
        let vls = MockVlsProvider::default();
        // 50k sats = 50,000,000 msat → hot path
        let (sig, strategy) = vls.sign_htlc(&[0x01; 32], 50_000_000).await.unwrap();
        assert_eq!(sig.len(), 64);
        assert!(matches!(strategy, SigningStrategy::HotPath));
    }

    #[tokio::test]
    async fn test_mock_htlc_cold_path() {
        let vls = MockVlsProvider::default();
        // 600k sats = 600,000,000 msat → cold path (above 500k threshold)
        let (sig, strategy) = vls.sign_htlc(&[0x01; 32], 600_000_000).await.unwrap();
        assert_eq!(sig.len(), 64);
        assert!(matches!(strategy, SigningStrategy::ColdPath));
    }

    #[tokio::test]
    async fn test_mock_htlc_boundary() {
        let vls = MockVlsProvider::default();
        // Exactly 500k sats = 500,000,000 msat → cold path (not less than threshold)
        let (_, strategy) = vls.sign_htlc(&[0x01; 32], 500_000_000).await.unwrap();
        assert!(matches!(strategy, SigningStrategy::ColdPath));

        // 499,999 sats = 499,999,000 msat → hot path (below threshold)
        let (_, strategy) = vls.sign_htlc(&[0x01; 32], 499_999_000).await.unwrap();
        assert!(matches!(strategy, SigningStrategy::HotPath));
    }

    #[tokio::test]
    async fn test_custom_threshold() {
        // Custom threshold: 10k sats = 10,000,000 msat
        let vls = MockVlsProvider::new(10_000_000);

        // 5k sats → hot path (below 10k threshold)
        let (_, strategy) = vls.sign_htlc(&[0x01; 32], 5_000_000).await.unwrap();
        assert!(matches!(strategy, SigningStrategy::HotPath));

        // 15k sats → cold path (above 10k threshold)
        let (_, strategy) = vls.sign_htlc(&[0x01; 32], 15_000_000).await.unwrap();
        assert!(matches!(strategy, SigningStrategy::ColdPath));
    }
}
