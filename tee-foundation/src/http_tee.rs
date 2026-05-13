//! HttpTeeServer - HTTP transport layer for TEE attestation
//!
//! Provides HTTP handlers for:
//! - Remote attestation challenge-response (JWT-protected)
//! - TEE status check (JWT-protected)
//!
//! Part of feature 039: Confidential Compute Integration.
//!
//! ## Architecture
//!
//! Follows the five-layer architecture (ADR-001):
//! - Transport Layer: This module (extracts HTTP data, delegates to TEE providers)
//! - TEE Providers: HardwareTrust + VlsProvider traits in src/tee/
//!
//! ## Route Structure
//!
//! JWT-Protected (v2 API):
//! - POST /api/v2/tee/attest - Remote attestation challenge-response
//! - GET  /api/v2/tee/status - TEE provider status

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use crate::dto::ApiResponse;
use crate::error::AppError;
use crate::state::AppState;
use crate::tee::traits::AttestationChallenge;

/// Request body for remote attestation challenge.
#[derive(Debug, Clone, Deserialize)]
pub struct AttestationRequest {
    /// Node ID of the requesting peer
    pub requester_node_id: String,
    /// Base64-encoded 32-byte challenge nonce
    pub challenge_nonce: String,
}

/// Response body for remote attestation.
#[derive(Debug, Clone, Serialize)]
pub struct AttestationResponseDto {
    /// Whether the attestation is backed by real hardware
    pub is_hardware_backed: bool,
    /// Base64-encoded kernel hash measurement
    pub kernel_hash: String,
    /// Base64-encoded ECDSA signature
    pub signature: String,
    /// Hardware platform identifier
    pub platform: String,
    /// Firmware version
    pub firmware_version: String,
    /// Number of TPM PCR values included
    pub pcr_count: usize,
}

/// Response body for TEE status check.
#[derive(Debug, Clone, Serialize)]
pub struct TeeStatusResponse {
    /// Whether TEE is enabled
    pub enabled: bool,
    /// Whether hardware trust provider is available
    pub hardware_trust_available: bool,
    /// Whether VLS provider is available
    pub vls_available: bool,
    /// Whether providers are hardware-backed
    pub is_hardware_backed: bool,
    /// Watchdog status: "healthy", "warning", "degraded", "stopped", or null if not running
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watchdog_status: Option<String>,
    /// Circuit breaker state: "closed", "open", "half_open", or null if not initialized
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_breaker_state: Option<String>,
    /// Number of consecutive hardware failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_breaker_failure_count: Option<u32>,
    /// Number of channels in degraded state (T-705 post-restart verification)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_channel_count: Option<u32>,
}

pub struct HttpTeeServer;

impl HttpTeeServer {
    /// Register JWT-protected TEE routes under /api/v2/tee/
    pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
        router
            .route("/tee/attest", post(Self::http_attest))
            .route("/tee/status", get(Self::http_status))
    }

    /// POST /api/v2/tee/attest - Remote attestation challenge-response
    ///
    /// A remote node sends a challenge nonce, this node responds with
    /// boot measurements and ECDSA signature from TEE hardware (or mock).
    async fn http_attest(
        State(state): State<Arc<AppState>>,
        Json(request): Json<AttestationRequest>,
    ) -> Result<Json<ApiResponse<AttestationResponseDto>>, AppError> {
        info!(
            target: "node_backend::tee",
            requester = %request.requester_node_id,
            "Processing remote attestation challenge"
        );

        let trust = state.tee_trust.as_ref().ok_or_else(|| {
            AppError::BadRequest("TEE not enabled on this node".to_string())
        })?;

        // Decode base64 nonce
        use base64::Engine;
        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(&request.challenge_nonce)
            .map_err(|e| AppError::Validation(format!("Invalid base64 nonce: {}", e)))?;

        if nonce_bytes.len() != 32 {
            return Err(AppError::Validation(format!(
                "Challenge nonce must be 32 bytes, got {}",
                nonce_bytes.len()
            )));
        }

        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&nonce_bytes);

        let challenge = AttestationChallenge {
            requester_node_id: request.requester_node_id,
            challenge_nonce: nonce,
            timestamp: std::time::SystemTime::now(),
        };

        let response = trust.attest(challenge).await.map_err(|e| {
            AppError::Internal(format!("Attestation failed: {}", e))
        })?;

        let dto = AttestationResponseDto {
            is_hardware_backed: response.boot_measurement.is_hardware_backed,
            kernel_hash: base64::engine::general_purpose::STANDARD
                .encode(response.boot_measurement.kernel_hash),
            signature: base64::engine::general_purpose::STANDARD
                .encode(&response.boot_measurement.signature),
            platform: response.platform_info.platform.clone(),
            firmware_version: response.platform_info.firmware_version.clone(),
            pcr_count: response.platform_info.tpm_pcr_values.len(),
        };

        Ok(Json(ApiResponse::success(dto, "Attestation complete")))
    }

    /// GET /api/v2/tee/status - Check TEE provider status
    async fn http_status(
        State(state): State<Arc<AppState>>,
    ) -> Result<Json<ApiResponse<TeeStatusResponse>>, AppError> {
        let hardware_trust_available = state.tee_trust.is_some();
        let vls_available = state.tee_vls.is_some();
        let is_hardware_backed = state
            .tee_vls
            .as_ref()
            .map(|v| v.is_hardware_backed())
            .unwrap_or(false);

        // Read watchdog status if handle exists
        let watchdog_status = state.watchdog_handle.as_ref().map(|h| {
            match h.status() {
                crate::tee::WatchdogStatus::Healthy => "healthy".to_string(),
                crate::tee::WatchdogStatus::Warning { consecutive_misses } => {
                    format!("warning (misses: {})", consecutive_misses)
                }
                crate::tee::WatchdogStatus::Degraded => "degraded".to_string(),
                crate::tee::WatchdogStatus::Stopped => "stopped".to_string(),
            }
        });

        // Read circuit breaker state if available
        let (circuit_breaker_state, circuit_breaker_failure_count) =
            if let Some(ref cb) = state.tee_circuit_breaker {
                (
                    Some(cb.state().to_string()),
                    Some(cb.failure_count()),
                )
            } else {
                (None, None)
            };

        // Live channel verification (T-705)
        // NOTE: LDK node extracted to builtin app — channel verification now lives there.
        // This endpoint reports TEE provider status only; channel health is separate.
        let degraded_channel_count: Option<u32> = None;

        let status = TeeStatusResponse {
            enabled: hardware_trust_available || vls_available,
            hardware_trust_available,
            vls_available,
            is_hardware_backed,
            watchdog_status,
            circuit_breaker_state,
            circuit_breaker_failure_count,
            degraded_channel_count,
        };

        Ok(Json(ApiResponse::success(status, "TEE status")))
    }
}
