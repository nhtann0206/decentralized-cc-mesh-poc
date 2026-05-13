/// TEE-specific error types.
///
/// Covers hardware communication, attestation, and signing failures.
/// All variants logged with target `node_backend::tee`.
#[derive(Debug, thiserror::Error)]
pub enum TeeError {
    #[error("TEE not enabled")]
    NotEnabled,

    #[error("Hardware error: {0}")]
    HardwareError(String),

    #[error("Attestation failed: {0}")]
    AttestationFailed(String),

    #[error("Signing error: {0}")]
    SigningError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("I2C communication error: {0}")]
    I2cError(String),

    #[error("Watchdog timeout")]
    WatchdogTimeout,
}
