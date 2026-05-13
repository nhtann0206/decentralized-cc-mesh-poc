//! Trusted Execution Environment (TEE) module
//!
//! Provides hardware-backed security via trait abstraction:
//! - `HardwareTrust`: Boot attestation, TPM PCR, watchdog heartbeat
//! - `VlsProvider`: Hybrid signing (hot/cold path) for Lightning operations
//! - `SecureMemory`: mlock-protected memory for sensitive key material
//! - `HealthProbe`: Subsystem health probing for advanced watchdog
//! - `AttestationRegistry`: Public key registry for attestation verification
//!
//! Configuration: `TEE_ENABLED=false` (default) has zero impact on existing code.
//!
//! LDK integration (M5B):
//! - `ldk_signer_adapter`: VlsSignerProvider + signer strategy logic (backend-side)
//! - `recovery`: Channel verification pure functions (backend-side)
//! - `BackendTeeSignerFactory` (ldk_node::TeeSignerFactory impl): lives in
//!   `builtin-apps/ldk-node` where it has access to `ldk_node::Builder`

pub mod attestation_registry;
pub mod circuit_breaker;
pub mod cobs;
pub mod config;
pub mod error;
pub mod health;
pub mod i2c_actor;
pub mod metrics;
pub mod probes;
pub mod providers;
pub mod secure_memory;
pub mod serial_transport;
pub mod sev_snp_verifier;
pub mod traits;
pub mod watchdog;
pub mod seed_encryption;
pub mod wrap_key;
pub mod ldk_signer_adapter;
pub mod recovery;

pub use circuit_breaker::{CircuitState, TeeCircuitBreaker};
pub use wrap_key::{WrapKeyProvider, MockWrapKeyProvider, OpteeWrapKeyProvider, new_wrap_key_provider};
pub use config::{TeeConfig, TeeProviderType};
pub use error::TeeError;
pub use traits::{AttestationChallenge, AttestationResponse, BootAttestation, HardwareTrust, PlatformInfo, SigningStrategy, VlsProvider};
pub use watchdog::{WatchdogHandle, WatchdogStatus, spawn_mock_watchdog, spawn_watchdog, MockWatchdogTransport};
pub use secure_memory::SecureMemory;
pub use health::{AggregateHealth, HealthProbe, HealthAggregator, SubsystemHealth};
pub use attestation_registry::{AttestationRegistry, MockAttestationRegistry, DeviceCertificate, AttestationVerifyResult};
pub use probes::{CircuitBreakerProbe, DatabaseHealthProbe, LdkNodeHealthProbe, WatchdogHealthProbe};
pub use providers::LinuxI2cHandler;
pub use serial_transport::SerialWatchdogTransport;
pub use ldk_signer_adapter::{VlsSignerProvider, SignerStrategy, derive_signer_strategy};
pub use recovery::{ChannelVerification, ChannelVerificationStatus, ChannelRecoveryReport, classify_channel, expected_strategy, build_recovery_report};

use providers::{MockHardwareTrust, MockVlsProvider};
use std::sync::Arc;

/// Harden the current process against memory inspection by same-user processes.
///
/// Sets `PR_SET_DUMPABLE=0` which:
/// - Changes `/proc/pid/` ownership to `root:root` (blocks same-UID reads)
/// - Prevents `ptrace(PTRACE_ATTACH)` from same-user processes
/// - Used by ssh-agent, gpg-agent for the same purpose
///
/// NOTE: Does NOT protect against root with `CAP_SYS_PTRACE`.
/// TrustZone (OP-TEE) is the only real boundary against privileged attackers.
pub fn harden_process_memory() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl(PR_SET_DUMPABLE, 0) is a standard process attribute change.
        // It only affects the calling process's dumpable flag.
        let result = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) };
        if result == 0 {
            tracing::info!(
                target: "node_backend::tee",
                "Process hardened: PR_SET_DUMPABLE=0 (blocks same-user /proc/pid/mem reads)"
            );
        } else {
            tracing::warn!(
                target: "node_backend::tee",
                error = %std::io::Error::last_os_error(),
                "Failed to set PR_SET_DUMPABLE=0 — same-user processes may read /proc/pid/mem"
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::debug!(
            target: "node_backend::tee",
            "PR_SET_DUMPABLE not available on this platform (non-Linux)"
        );
    }
}

/// Initialize TEE providers based on configuration.
///
/// Returns `(None, None)` if TEE is disabled.
/// Returns mock providers if `TEE_PROVIDER=mock`.
/// Returns error if `TEE_PROVIDER=hardware` (Phase 3 - not yet implemented).
pub fn initialize_providers(
    config: &TeeConfig,
) -> Result<(Option<Arc<dyn HardwareTrust>>, Option<Arc<dyn VlsProvider>>), TeeError> {
    if !config.enabled {
        tracing::info!(
            target: "node_backend::tee",
            "TEE disabled, skipping provider initialization"
        );
        return Ok((None, None));
    }

    // Harden process memory before initializing any key material
    harden_process_memory();

    match &config.provider {
        TeeProviderType::Mock => {
            tracing::info!(
                target: "node_backend::tee",
                provider = "mock",
                "Initializing mock TEE providers"
            );
            let trust: Arc<dyn HardwareTrust> = Arc::new(MockHardwareTrust::new());
            let vls: Arc<dyn VlsProvider> = Arc::new(MockVlsProvider::new(config.htlc_hot_path_threshold_msat));
            Ok((Some(trust), Some(vls)))
        }
        TeeProviderType::Hardware { .. } => {
            // Phase 3: Hardware providers not yet implemented
            Err(TeeError::ConfigError(
                "Hardware TEE provider not yet implemented (Phase 3)".to_string(),
            ))
        }
    }
}
