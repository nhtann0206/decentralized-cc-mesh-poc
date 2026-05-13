use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::error::TeeError;

/// Boot attestation data from hardware (TPM + ATECC608A via I2C).
///
/// Key material (`nonce`, `kernel_hash`, `signature`) is zeroized on drop.
#[derive(Clone, Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct BootAttestation {
    /// Random nonce from attestation challenge
    pub nonce: [u8; 32],
    /// SHA-256 hash of kernel image
    pub kernel_hash: [u8; 32],
    /// ECDSA signature over (kernel_hash + nonce), 64 bytes ECDSA
    pub signature: Vec<u8>,
    /// When attestation was performed
    #[zeroize(skip)]
    pub verified_at: SystemTime,
    /// `false` for mock, `true` for hardware (ATECC608A + TPM)
    pub is_hardware_backed: bool,
}

/// Challenge sent by remote node for attestation verification.
///
/// Challenge nonce is zeroized on drop to prevent replay data leakage.
#[derive(Clone, Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct AttestationChallenge {
    /// Node ID of the requesting peer
    pub requester_node_id: String,
    /// Fresh nonce to prevent replay attacks
    pub challenge_nonce: [u8; 32],
    /// When the challenge was created
    #[zeroize(skip)]
    pub timestamp: SystemTime,
}

/// Platform information included in attestation responses.
#[derive(Clone, Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct PlatformInfo {
    /// Hardware platform (e.g., "radxa-zero-3w", "mock")
    pub platform: String,
    /// Platform/OS version
    pub firmware_version: String,
    /// TPM PCR values (PCR-0 through PCR-23, only populated ones included)
    #[zeroize(skip)]
    pub tpm_pcr_values: Vec<(u8, [u8; 32])>,
}

/// Response to a remote attestation challenge.
///
/// All sensitive data (boot measurement, signatures) is zeroized on drop.
#[derive(Clone, Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct AttestationResponse {
    /// Boot measurement data
    pub boot_measurement: BootAttestation,
    /// Hardware platform details
    pub platform_info: PlatformInfo,
}

/// Signing strategy selection for hybrid hot/cold path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SigningStrategy {
    /// Hot keys in RAM (zeroize-protected) for fast HTLC forwarding (below threshold, default 500k sats)
    HotPath,
    /// Cold keys in ATECC608A for commitment transactions and channel closes
    ColdPath,
}

/// Hardware trust abstraction for boot verification and attestation.
///
/// Two implementations:
/// - `MockHardwareTrust`: Development (returns fake attestation, `is_hardware_backed: false`)
/// - `RadxaHardwareTrust` (Phase 3): Production (direct I2C to ATECC608A + TPM)
#[async_trait]
pub trait HardwareTrust: Send + Sync {
    /// Get boot attestation from hardware (TPM + ATECC608A).
    async fn get_boot_attestation(&self) -> Result<BootAttestation, TeeError>;

    /// Read TPM PCR value for boot measurements.
    async fn read_tpm_pcr(&self, pcr: u8) -> Result<[u8; 32], TeeError>;

    /// Send watchdog heartbeat (must complete within 30s, timeout at 60s).
    async fn send_heartbeat(&self) -> Result<(), TeeError>;

    /// Respond to a remote attestation challenge with signed measurements.
    async fn attest(
        &self,
        challenge: AttestationChallenge,
    ) -> Result<AttestationResponse, TeeError>;

    /// Returns whether this provider is backed by real hardware (ATECC608A + TPM).
    /// Mock providers return `false`; production providers return `true`.
    fn is_hardware_backed(&self) -> bool;
}

/// VLS (Validating Lightning Signer) provider for hybrid signing.
///
/// Two implementations:
/// - `MockVlsProvider`: Software signing for development
/// - `AteccVlsProvider` (Phase 4): ATECC608A hardware signing via I2C
#[async_trait]
pub trait VlsProvider: Send + Sync {
    /// Sign arbitrary data (returns DER-encoded ECDSA signature).
    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, TeeError>;

    /// Sign a commitment transaction (always cold path via ATECC608A).
    async fn sign_commitment(&self, tx_hash: &[u8; 32]) -> Result<Vec<u8>, TeeError>;

    /// Sign an HTLC (hot path if below threshold, cold path otherwise).
    async fn sign_htlc(
        &self,
        htlc_hash: &[u8; 32],
        amount_msat: u64,
    ) -> Result<(Vec<u8>, SigningStrategy), TeeError>;

    /// Returns whether this provider is backed by real hardware.
    fn is_hardware_backed(&self) -> bool;
}
