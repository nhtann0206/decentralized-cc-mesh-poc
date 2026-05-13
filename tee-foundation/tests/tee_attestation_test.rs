//! Integration test: Alice ↔ Bob mock TEE attestation
//!
//! Tests the full attestation flow between two mock nodes:
//! 1. Alice initializes mock TEE providers
//! 2. Bob sends attestation challenge to Alice
//! 3. Alice responds with boot measurements
//! 4. Bob verifies the response

use lightning_node_backend::tee::{
    self, AttestationChallenge, AttestationVerifyResult, MockAttestationRegistry,
    AttestationRegistry, SigningStrategy,
    attestation_registry::create_test_certificate,
    config::{TeeConfig, TeeProviderType, DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT},
};
use std::time::SystemTime;

#[tokio::test]
async fn test_alice_bob_mock_attestation() {
    // Alice: Initialize mock TEE providers
    let alice_config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (alice_trust, alice_vls) = tee::initialize_providers(&alice_config).unwrap();
    let alice_trust = alice_trust.expect("Alice trust should be Some");
    let alice_vls = alice_vls.expect("Alice VLS should be Some");

    // Bob: Initialize mock TEE providers
    let bob_config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (bob_trust, _bob_vls) = tee::initialize_providers(&bob_config).unwrap();
    let bob_trust = bob_trust.expect("Bob trust should be Some");

    // Bob creates attestation challenge for Alice
    let challenge = AttestationChallenge {
        requester_node_id: "bob-mock-node-id".to_string(),
        challenge_nonce: [0x01; 32],
        timestamp: SystemTime::now(),
    };

    // Alice responds to Bob's challenge
    let response = alice_trust.attest(challenge).await.unwrap();

    // Verify response fields
    assert!(!response.boot_measurement.is_hardware_backed, "Mock should not be hardware-backed");
    assert_eq!(response.platform_info.platform, "mock");
    assert!(!response.platform_info.tpm_pcr_values.is_empty(), "Should have PCR values");
    assert_eq!(response.boot_measurement.signature.len(), 64, "Signature should be 64 bytes");

    // Bob can also get attestation
    let bob_attestation = bob_trust.get_boot_attestation().await.unwrap();
    assert!(!bob_attestation.is_hardware_backed);

    // Verify VLS signing works
    assert!(!alice_vls.is_hardware_backed());
}

#[tokio::test]
async fn test_tee_disabled_returns_none() {
    let config = TeeConfig {
        enabled: false,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (trust, vls) = tee::initialize_providers(&config).unwrap();
    assert!(trust.is_none(), "TEE disabled should return None trust");
    assert!(vls.is_none(), "TEE disabled should return None VLS");
}

#[tokio::test]
async fn test_htlc_signing_strategy_selection() {
    let config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (_, vls) = tee::initialize_providers(&config).unwrap();
    let vls = vls.unwrap();

    // Small HTLC (50k sats = 50,000,000 msat) → hot path
    let (sig, strategy) = vls.sign_htlc(&[0x01; 32], 50_000_000).await.unwrap();
    assert!(!sig.is_empty());
    assert!(matches!(strategy, SigningStrategy::HotPath));

    // Large HTLC (600k sats = 600,000,000 msat) → cold path (above 500k default threshold)
    let (sig, strategy) = vls.sign_htlc(&[0x01; 32], 600_000_000).await.unwrap();
    assert!(!sig.is_empty());
    assert!(matches!(strategy, SigningStrategy::ColdPath));
}

#[tokio::test]
async fn test_commitment_always_cold_path() {
    let config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (_, vls) = tee::initialize_providers(&config).unwrap();
    let vls = vls.unwrap();

    // Commitment transactions always use cold path
    let sig = vls.sign_commitment(&[0x01; 32]).await.unwrap();
    assert_eq!(sig.len(), 64);
}

#[tokio::test]
async fn test_hardware_provider_not_implemented() {
    let config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Hardware {
            i2c: "/dev/i2c-1".to_string(),
            atecc_addr: 0x60,
        },
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let result = tee::initialize_providers(&config);
    assert!(result.is_err(), "Hardware provider should fail (Phase 3)");
}

// ── Alice-Bob Pre-Hardware Test Cases ──────────────────────────────────

/// A2: Unknown device attestation → rejected.
/// Proves trust boundary enforcement: unregistered nodes cannot pass verification.
#[tokio::test]
async fn test_unknown_device_attestation_rejected() {
    let registry = MockAttestationRegistry::new();

    // Verify attestation from an unregistered node
    let result = registry
        .verify_attestation("unknown-node-xyz", b"some-data", &[0x42; 64])
        .await;

    assert_eq!(result, AttestationVerifyResult::UnknownDevice);
    assert!(!result.is_valid());
}

/// A3: Empty/invalid signature → rejected.
/// Proves signature validation: even a registered node fails with empty signature.
#[tokio::test]
async fn test_empty_signature_rejected() {
    let registry = MockAttestationRegistry::new();
    let cert = create_test_certificate("alice-node-001", "Alice");
    registry.register(cert).await.unwrap();

    // Empty signature → InvalidSignature
    let result = registry
        .verify_attestation("alice-node-001", b"attestation-data", &[])
        .await;
    assert_eq!(result, AttestationVerifyResult::InvalidSignature);

    // Empty signed_data → InvalidSignature
    let result = registry
        .verify_attestation("alice-node-001", b"", &[0x42; 64])
        .await;
    assert_eq!(result, AttestationVerifyResult::InvalidSignature);
}

/// A4: Full Alice-Bob attestation flow with registry-based trust verification.
///
/// Business logic verified:
/// 1. Alice registers her device certificate in the shared registry
/// 2. Bob sends attestation challenge to Alice
/// 3. Alice responds with boot measurements
/// 4. Bob verifies Alice's response via the registry (public key lookup + signature check)
/// 5. An unregistered node ("eve") fails verification
#[tokio::test]
async fn test_alice_bob_full_attestation_with_registry() {
    // Shared registry (in production: gossip-distributed or CA-signed)
    let registry = MockAttestationRegistry::new();

    // Alice registers her device certificate
    let alice_cert = create_test_certificate("alice-node-001", "Alice's Radxa");
    registry.register(alice_cert).await.unwrap();

    // Alice initializes TEE
    let alice_config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (alice_trust, _) = tee::initialize_providers(&alice_config).unwrap();
    let alice_trust = alice_trust.unwrap();

    // Bob creates attestation challenge
    let challenge = AttestationChallenge {
        requester_node_id: "bob-node-002".to_string(),
        challenge_nonce: [0xBB; 32],
        timestamp: SystemTime::now(),
    };

    // Alice responds
    let response = alice_trust.attest(challenge).await.unwrap();

    // Bob verifies Alice's attestation via registry
    let verify_result = registry
        .verify_attestation(
            "alice-node-001",
            &response.boot_measurement.kernel_hash,
            &response.boot_measurement.signature,
        )
        .await;
    assert!(
        verify_result.is_valid(),
        "Registered Alice should pass attestation: got {:?}",
        verify_result
    );

    // Eve (unregistered) fails
    let eve_result = registry
        .verify_attestation(
            "eve-node-666",
            &response.boot_measurement.kernel_hash,
            &response.boot_measurement.signature,
        )
        .await;
    assert_eq!(
        eve_result,
        AttestationVerifyResult::UnknownDevice,
        "Unregistered Eve must be rejected"
    );
}

/// A5: Revoked node attestation → rejected.
///
/// Business logic verified: Certificate lifecycle — a node that was once trusted
/// becomes untrusted after revocation (key rotation or decommissioning).
#[tokio::test]
async fn test_revoked_node_attestation_rejected() {
    let registry = MockAttestationRegistry::new();

    // Register and verify
    let cert = create_test_certificate("compromised-node", "Compromised");
    registry.register(cert).await.unwrap();

    let valid = registry
        .verify_attestation("compromised-node", b"data", &[0x42; 64])
        .await;
    assert!(valid.is_valid(), "Should be valid before revocation");

    // Revoke the certificate
    let revoked = registry.revoke("compromised-node").await.unwrap();
    assert!(revoked, "Should return true for successful revocation");

    // After revocation → UnknownDevice
    let post_revoke = registry
        .verify_attestation("compromised-node", b"data", &[0x42; 64])
        .await;
    assert_eq!(
        post_revoke,
        AttestationVerifyResult::UnknownDevice,
        "Revoked node must fail attestation verification"
    );
}

/// A6: PCR-10 value in attestation response matches expected kernel hash.
///
/// Business logic verified: Boot integrity — the attestation response includes
/// TPM PCR values that can be compared against a known-good baseline.
/// In production, PCR mismatch → reject attestation (tampered kernel).
#[tokio::test]
async fn test_pcr_values_in_attestation_match_expected() {
    let config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (trust, _) = tee::initialize_providers(&config).unwrap();
    let trust = trust.unwrap();

    let challenge = AttestationChallenge {
        requester_node_id: "verifier-node".to_string(),
        challenge_nonce: [0xAA; 32],
        timestamp: SystemTime::now(),
    };

    let response = trust.attest(challenge).await.unwrap();

    // Verify PCR-10 (kernel hash) is present and matches expected mock value
    let pcr10 = response
        .platform_info
        .tpm_pcr_values
        .iter()
        .find(|(idx, _)| *idx == 10);
    assert!(pcr10.is_some(), "PCR-10 must be present in attestation");
    assert_eq!(
        pcr10.unwrap().1,
        [0xAB; 32],
        "PCR-10 must match expected kernel hash (mock = 0xAB)"
    );

    // Verify kernel_hash in boot_measurement matches PCR-10
    assert_eq!(
        response.boot_measurement.kernel_hash,
        [0xAB; 32],
        "Boot measurement kernel_hash must match PCR-10"
    );

    // Simulate PCR mismatch detection (verifier side)
    let expected_kernel_hash: [u8; 32] = [0xAB; 32];
    let tampered_hash: [u8; 32] = [0xFF; 32];
    assert_eq!(
        response.boot_measurement.kernel_hash, expected_kernel_hash,
        "Valid kernel hash should match"
    );
    assert_ne!(
        response.boot_measurement.kernel_hash, tampered_hash,
        "Tampered hash must NOT match"
    );
}

/// A7: HTLC threshold boundary — exact boundary behavior at default 500k sats.
///
/// Business logic verified: Hot/cold path selection at the exact threshold boundary.
/// - 499,999,999 msat (just below 500k) → HotPath
/// - 500,000,000 msat (exactly 500k) → ColdPath
/// - 500,000,001 msat (just above 500k) → ColdPath
#[tokio::test]
async fn test_htlc_threshold_boundary_at_default() {
    let config = TeeConfig {
        enabled: true,
        provider: TeeProviderType::Mock,
        htlc_hot_path_threshold_msat: DEFAULT_HTLC_HOT_PATH_THRESHOLD_MSAT,
    };
    let (_, vls) = tee::initialize_providers(&config).unwrap();
    let vls = vls.unwrap();

    // Just below threshold: 499,999,999 msat → HotPath
    let (_, strategy) = vls.sign_htlc(&[0x01; 32], 499_999_999).await.unwrap();
    assert!(
        matches!(strategy, SigningStrategy::HotPath),
        "499,999,999 msat (just below 500k) must use HotPath"
    );

    // Exactly at threshold: 500,000,000 msat → ColdPath (not less than)
    let (_, strategy) = vls.sign_htlc(&[0x01; 32], 500_000_000).await.unwrap();
    assert!(
        matches!(strategy, SigningStrategy::ColdPath),
        "500,000,000 msat (exactly 500k) must use ColdPath"
    );

    // Just above threshold: 500,000,001 msat → ColdPath
    let (_, strategy) = vls.sign_htlc(&[0x01; 32], 500_000_001).await.unwrap();
    assert!(
        matches!(strategy, SigningStrategy::ColdPath),
        "500,000,001 msat (just above 500k) must use ColdPath"
    );
}
