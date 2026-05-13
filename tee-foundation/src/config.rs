/// TEE provider type configuration.
#[derive(Clone, Debug, PartialEq)]
pub enum TeeProviderType {
    /// Software mock for development/CI (no hardware required)
    Mock,
    /// Real hardware: Radxa Zero 3W + ATECC608A + TPM 2.0 (direct I2C)
    Hardware {
        i2c: String,
        atecc_addr: u8,
    },
}

/// Default HTLC hot path threshold: 500,000 sats = 500,000,000 msat (~$500 at $100k BTC).
///
/// HTLCs below this value use hot path (RAM keys, low latency).
/// HTLCs at or above this value use cold path (ATECC608A hardware signing).
///
/// Rationale: Most channels are < $100 (typical LN channel size). Setting the threshold
/// at $500 ensures performance for everyday small payments while protecting larger balances
/// with hardware signing. Configurable via `TEE_HTLC_THRESHOLD_MSAT` env var.
pub const DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT: u64 = 500_000_000;

/// TEE configuration loaded from environment variables.
///
/// - `TEE_ENABLED`: "true" or "false" (default: "false")
/// - `TEE_PROVIDER`: "mock" (default) or "hardware"
/// - `TEE_TPM_I2C`: I2C device path (default: "/dev/i2c-1")
/// - `TEE_ATECC_ADDR`: ATECC608A I2C address in hex (default: "0x60")
/// - `TEE_HTLC_THRESHOLD_MSAT`: Hot/cold path threshold in msat (default: 500,000,000)
#[derive(Clone, Debug)]
pub struct TeeConfig {
    pub enabled: bool,
    pub provider: TeeProviderType,
    /// HTLC amount threshold (in millisatoshis) for hot/cold path selection.
    /// HTLCs below this value use hot path (RAM keys), above use cold path (ATECC608A).
    pub htlc_hot_path_threshold_msat: u64,
}

impl TeeConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("TEE_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            == "true";

        let provider = match std::env::var("TEE_PROVIDER").as_deref() {
            Ok("hardware") => TeeProviderType::Hardware {
                i2c: std::env::var("TEE_TPM_I2C")
                    .unwrap_or_else(|_| "/dev/i2c-1".into()),
                atecc_addr: std::env::var("TEE_ATECC_ADDR")
                    .ok()
                    .and_then(|s| {
                        u8::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                    })
                    .unwrap_or(0x60),
            },
            _ => TeeProviderType::Mock,
        };

        let htlc_hot_path_threshold_msat = std::env::var("TEE_HTLC_THRESHOLD_MSAT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT);

        Self {
            enabled,
            provider,
            htlc_hot_path_threshold_msat,
        }
    }
}

