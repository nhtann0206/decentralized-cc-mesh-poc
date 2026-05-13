use async_trait::async_trait;
use rand::{thread_rng, RngCore};
use std::collections::HashMap;
use std::time::SystemTime;

use crate::tee::error::TeeError;
use crate::tee::traits::{
    AttestationChallenge, AttestationResponse, BootAttestation, HardwareTrust, PlatformInfo,
};

/// Mock hardware trust provider for development and CI/CD.
///
/// Returns realistic but fake attestation data with `is_hardware_backed: false`.
/// No hardware dependencies required.
pub struct MockHardwareTrust {
    fake_pcr: HashMap<u8, [u8; 32]>,
}

impl MockHardwareTrust {
    pub fn new() -> Self {
        let mut fake_pcr = HashMap::new();
        // PCR-10: Simulated kernel hash measurement
        fake_pcr.insert(10, [0xAB; 32]);
        // PCR-0: Simulated firmware measurement
        fake_pcr.insert(0, [0xCD; 32]);

        Self { fake_pcr }
    }
}

impl Default for MockHardwareTrust {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HardwareTrust for MockHardwareTrust {
    async fn get_boot_attestation(&self) -> Result<BootAttestation, TeeError> {
        // Generate random nonce for each attestation (meaningful challenge-response)
        let mut nonce = [0u8; 32];
        thread_rng().fill_bytes(&mut nonce);

        // Simulated signature over (nonce || kernel_hash) — not cryptographically valid
        // but demonstrates the protocol flow with unique data per call
        let kernel_hash = self.fake_pcr.get(&10).copied().unwrap_or([0; 32]);
        let mut signature = Vec::with_capacity(64);
        signature.extend_from_slice(&nonce[..32]);
        signature.extend_from_slice(&kernel_hash[..32]);

        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            nonce_prefix = %hex::encode(&nonce[..4]),
            "Generating mock boot attestation with random nonce"
        );

        Ok(BootAttestation {
            nonce,
            kernel_hash,
            signature,
            verified_at: SystemTime::now(),
            is_hardware_backed: false,
        })
    }

    async fn read_tpm_pcr(&self, pcr: u8) -> Result<[u8; 32], TeeError> {
        tracing::debug!(
            target: "node_backend::tee",
            provider = "mock",
            pcr_index = pcr,
            "Reading mock TPM PCR"
        );

        self.fake_pcr
            .get(&pcr)
            .copied()
            .ok_or_else(|| TeeError::HardwareError(format!("PCR-{} not available in mock", pcr)))
    }

    async fn send_heartbeat(&self) -> Result<(), TeeError> {
        tracing::debug!(
            target: "node_backend::tee",
            provider = "mock",
            "Mock watchdog heartbeat sent"
        );
        Ok(())
    }

    async fn attest(
        &self,
        challenge: AttestationChallenge,
    ) -> Result<AttestationResponse, TeeError> {
        tracing::info!(
            target: "node_backend::tee",
            provider = "mock",
            requester = %challenge.requester_node_id,
            "Processing mock attestation challenge"
        );

        let boot = self.get_boot_attestation().await?;

        let pcr_values: Vec<(u8, [u8; 32])> = self
            .fake_pcr
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();

        Ok(AttestationResponse {
            boot_measurement: boot,
            platform_info: PlatformInfo {
                platform: "mock".to_string(),
                firmware_version: "0.0.0-mock".to_string(),
                tpm_pcr_values: pcr_values,
            },
        })
    }

    fn is_hardware_backed(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_boot_attestation() {
        let trust = MockHardwareTrust::new();
        let attestation = trust.get_boot_attestation().await.unwrap();

        assert!(!attestation.is_hardware_backed);
        assert_eq!(attestation.kernel_hash, [0xAB; 32]);
        // Nonce should be random (not all zeros, not the old fixed 0x42)
        assert_ne!(attestation.nonce, [0u8; 32]);
        // Signature should be derived from nonce + kernel_hash (64 bytes)
        assert_eq!(attestation.signature.len(), 64);

        // Two attestations should produce different nonces
        let attestation2 = trust.get_boot_attestation().await.unwrap();
        assert_ne!(attestation.nonce, attestation2.nonce);
    }

    #[tokio::test]
    async fn test_mock_tpm_pcr_read() {
        let trust = MockHardwareTrust::new();

        let pcr10 = trust.read_tpm_pcr(10).await.unwrap();
        assert_eq!(pcr10, [0xAB; 32]);

        let pcr0 = trust.read_tpm_pcr(0).await.unwrap();
        assert_eq!(pcr0, [0xCD; 32]);

        // Non-existent PCR should error
        let result = trust.read_tpm_pcr(15).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_attestation_challenge() {
        let trust = MockHardwareTrust::new();
        let challenge = AttestationChallenge {
            requester_node_id: "bob-node-id".to_string(),
            challenge_nonce: [0x01; 32],
            timestamp: SystemTime::now(),
        };

        let response = trust.attest(challenge).await.unwrap();
        assert!(!response.boot_measurement.is_hardware_backed);
        assert_eq!(response.platform_info.platform, "mock");
        assert!(!response.platform_info.tpm_pcr_values.is_empty());
    }
}
