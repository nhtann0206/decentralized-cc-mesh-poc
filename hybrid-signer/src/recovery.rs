//! TEE Channel State Recovery — Post-restart verification of channel signers.
//!
//! After systemd watchdog restarts the backend:
//! 1. LDK reloads channel state from `/var/lib/ldk`
//! 2. `WalletKeysManager` re-derives channel signers from entropy seed
//! 3. `TeeSignerFactory` creates `HybridSigner` instances (if TEE enabled)
//!
//! This module provides pure functions for verifying channel health and
//! determining signing strategy consistency.
//!
//! # Integration with Builtin-App Architecture
//!
//! The `verify_channels_post_restart()` function from the sandbox has been split:
//! - Pure logic (classify_channel, expected_strategy) → lives here in backend
//! - LDK channel enumeration → lives in builtin ldk-node app (has `ldk_node::Node` access)
//! - The builtin app calls these pure functions and returns results via CapabilityRouter

use serde::Serialize;

/// Result of verifying a single channel's signer state.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelVerification {
    pub channel_id: String,
    pub counterparty_node_id: String,
    pub channel_value_sats: u64,
    pub expected_strategy: &'static str,
    pub is_usable: bool,
    pub is_channel_ready: bool,
    pub status: ChannelVerificationStatus,
}

/// Verification status for a single channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ChannelVerificationStatus {
    Verified,
    PendingSync,
    Degraded { reason: String },
}

/// Aggregate report from post-restart channel verification.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelRecoveryReport {
    pub total_channels: usize,
    pub verified_count: usize,
    pub pending_sync_count: usize,
    pub degraded_count: usize,
    pub channels: Vec<ChannelVerification>,
}

/// Classify a channel's verification status based on its health indicators.
///
/// Pure function — testable without an LDK Node instance.
pub fn classify_channel(
    is_usable: bool,
    is_channel_ready: bool,
) -> ChannelVerificationStatus {
    if is_usable && is_channel_ready {
        ChannelVerificationStatus::Verified
    } else if !is_channel_ready {
        ChannelVerificationStatus::PendingSync
    } else {
        ChannelVerificationStatus::Degraded {
            reason: format!(
                "Channel ready but not usable (usable={}, ready={})",
                is_usable, is_channel_ready
            ),
        }
    }
}

/// Determine expected signing strategy for a given channel value.
///
/// Pure function — shared between backend and builtin ldk-node app.
pub fn expected_strategy(channel_value_sats: u64, threshold_msat: u64) -> &'static str {
    let channel_value_msat = channel_value_sats.saturating_mul(1000);
    if channel_value_msat < threshold_msat {
        "hot"
    } else {
        "cold"
    }
}

/// Build a recovery report from a list of channel data.
///
/// The builtin ldk-node app calls `node.list_channels()` and passes the data here.
/// This keeps LDK-specific types out of the backend.
pub fn build_recovery_report(
    channels: Vec<(String, String, u64, bool, bool)>, // (channel_id, counterparty, value_sats, is_usable, is_channel_ready)
    threshold_msat: u64,
) -> ChannelRecoveryReport {
    let mut verifications = Vec::with_capacity(channels.len());
    let mut verified = 0usize;
    let mut pending_sync = 0usize;
    let mut degraded = 0usize;

    for (channel_id, counterparty, value_sats, is_usable, is_channel_ready) in &channels {
        let strategy = expected_strategy(*value_sats, threshold_msat);
        let status = classify_channel(*is_usable, *is_channel_ready);

        match &status {
            ChannelVerificationStatus::Verified => verified += 1,
            ChannelVerificationStatus::PendingSync => pending_sync += 1,
            ChannelVerificationStatus::Degraded { reason } => {
                degraded += 1;
                tracing::warn!(
                    target: "node_backend::tee",
                    channel_id = %channel_id,
                    counterparty = %counterparty,
                    channel_value_sats = value_sats,
                    reason = %reason,
                    "Degraded channel detected during post-restart verification"
                );
            }
        }

        verifications.push(ChannelVerification {
            channel_id: channel_id.clone(),
            counterparty_node_id: counterparty.clone(),
            channel_value_sats: *value_sats,
            expected_strategy: strategy,
            is_usable: *is_usable,
            is_channel_ready: *is_channel_ready,
            status,
        });
    }

    tracing::info!(
        target: "node_backend::tee",
        total = channels.len(),
        verified,
        pending_sync,
        degraded,
        "Post-restart channel verification complete"
    );

    ChannelRecoveryReport {
        total_channels: channels.len(),
        verified_count: verified,
        pending_sync_count: pending_sync,
        degraded_count: degraded,
        channels: verifications,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tee::config::DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT;

    #[test]
    fn test_classify_verified() {
        assert_eq!(classify_channel(true, true), ChannelVerificationStatus::Verified);
    }

    #[test]
    fn test_classify_pending_sync() {
        assert_eq!(classify_channel(false, false), ChannelVerificationStatus::PendingSync);
        assert_eq!(classify_channel(true, false), ChannelVerificationStatus::PendingSync);
    }

    #[test]
    fn test_classify_ready_but_not_usable() {
        let status = classify_channel(false, true);
        assert!(matches!(status, ChannelVerificationStatus::Degraded { .. }));
    }

    #[test]
    fn test_expected_strategy_below_threshold() {
        assert_eq!(expected_strategy(250_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT), "hot");
    }

    #[test]
    fn test_expected_strategy_above_threshold() {
        assert_eq!(expected_strategy(600_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT), "cold");
    }

    #[test]
    fn test_expected_strategy_at_threshold() {
        assert_eq!(expected_strategy(500_000, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT), "cold");
    }

    #[test]
    fn test_expected_strategy_zero_threshold() {
        assert_eq!(expected_strategy(1, 0), "cold");
    }

    #[test]
    fn test_expected_strategy_max_threshold() {
        assert_eq!(expected_strategy(1_000_000, u64::MAX), "hot");
    }

    #[test]
    fn test_expected_strategy_overflow_protection() {
        let strategy = expected_strategy(u64::MAX, u64::MAX);
        assert_eq!(strategy, "cold");
    }

    #[test]
    fn test_build_recovery_report() {
        let channels = vec![
            ("ch1".into(), "peer1".into(), 250_000u64, true, true),
            ("ch2".into(), "peer2".into(), 600_000u64, true, false),
            ("ch3".into(), "peer3".into(), 100_000u64, false, true),
        ];
        let report = build_recovery_report(channels, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT);
        assert_eq!(report.total_channels, 3);
        assert_eq!(report.verified_count, 1);
        assert_eq!(report.pending_sync_count, 1);
        assert_eq!(report.degraded_count, 1);
    }
}
