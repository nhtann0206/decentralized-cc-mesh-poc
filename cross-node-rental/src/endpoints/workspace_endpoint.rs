//! WorkspaceEndpoint — Protocol-agnostic endpoint layer for VM rental
//!
//! Defines the public endpoint trait for cross-node workspace operations.
//! L402-protected: external nodes pay Lightning to access.
//!
//! Both `HttpWorkspaceClient` (caller side) and `WorkspaceEndpointImpl`
//! (handler side) implement `PublicWorkspaceEndpoint`, ensuring type safety
//! across the network boundary.

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AppError;
use crate::state::AppState;
use crate::workspace::models::{
    BalanceResponse, RawAttestationEvidence, SecureComputer, StartSessionRequest,
    StartSessionResponse, StopSessionResponse, WorkspaceSession,
};

/// Public workspace endpoint trait (L402-protected, for inter-node VM rental)
///
/// Alice's node calls these methods on Bob's node to browse marketplace,
/// rent VMs, and manage sessions. All methods are L402-gated — Alice pays
/// Lightning to access Bob's workspace API.
///
/// The trait is implemented by:
/// - `WorkspaceEndpointImpl` (server side — Bob handles the request)
/// - `HttpWorkspaceClient` (client side — Alice sends the request via L402HttpClient)
#[async_trait]
/// Public workspace endpoint trait (L402-protected, for inter-node VM rental).
///
/// `target_node_id` is the peer being called:
/// - Server impl (Bob handling request): uses it for ownership verification
/// - Client impl (Alice making request): uses it to resolve Bob's URL via
///   L402HttpClient's IP pool resolution
///
/// Follows the same pattern as `PublicFriendEndpoint`.
pub trait PublicWorkspaceEndpoint: Send + Sync {
    /// List available VMs for rent on the target node.
    async fn list_computers(
        &self,
        target_node_id: &str,
    ) -> Result<Vec<SecureComputer>, AppError>;

    /// Start a new VM rental session on the target node.
    async fn start_session(
        &self,
        target_node_id: &str,
        user_id: &str,
        request: StartSessionRequest,
    ) -> Result<StartSessionResponse, AppError>;

    /// Get current session status + billing info.
    async fn get_session(
        &self,
        target_node_id: &str,
        session_id: &str,
    ) -> Result<Option<WorkspaceSession>, AppError>;

    /// List all sessions for a user on the target node.
    async fn list_user_sessions(
        &self,
        target_node_id: &str,
        user_id: &str,
    ) -> Result<Vec<WorkspaceSession>, AppError>;

    /// Stop a running session on the target node.
    async fn stop_session(
        &self,
        target_node_id: &str,
        session_id: &str,
    ) -> Result<StopSessionResponse, AppError>;

    /// Get user's credit balance on the target node.
    async fn get_balance(
        &self,
        target_node_id: &str,
        user_id: &str,
    ) -> Result<BalanceResponse, AppError>;

    /// Phase A — fetch provider-level CC attestation as RAW evidence (the
    /// caller is expected to verify the AMD chain herself; no trust in the
    /// provider's verifier). Used pre-rent in the 4-step rent wizard.
    async fn get_provider_cc_attestation(
        &self,
        target_node_id: &str,
    ) -> Result<RawAttestationEvidence, AppError>;
}

/// Server-side implementation — delegates to WorkspaceService.
pub struct WorkspaceEndpointImpl {
    state: Arc<AppState>,
}

impl WorkspaceEndpointImpl {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    async fn get_service(
        &self,
    ) -> Result<Arc<crate::workspace::WorkspaceService>, AppError> {
        let guard = self.state.workspace_service.read().await;
        guard
            .clone()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))
    }
}

#[async_trait]
impl PublicWorkspaceEndpoint for WorkspaceEndpointImpl {
    async fn list_computers(
        &self,
        _target_node_id: &str,
    ) -> Result<Vec<SecureComputer>, AppError> {
        let svc = self.get_service().await?;
        Ok(svc.list_computers().await)
    }

    async fn start_session(
        &self,
        _target_node_id: &str,
        user_id: &str,
        request: StartSessionRequest,
    ) -> Result<StartSessionResponse, AppError> {
        let svc = self.get_service().await?;
        let (session_id, status) = svc
            .start_session(user_id, request, None)
            .await
            .map_err(|e| AppError::Internal(e))?;
        Ok(StartSessionResponse {
            session_id,
            status,
        })
    }

    async fn get_session(
        &self,
        _target_node_id: &str,
        session_id: &str,
    ) -> Result<Option<WorkspaceSession>, AppError> {
        let svc = self.get_service().await?;
        Ok(svc.get_session(session_id))
    }

    async fn list_user_sessions(
        &self,
        _target_node_id: &str,
        user_id: &str,
    ) -> Result<Vec<WorkspaceSession>, AppError> {
        let svc = self.get_service().await?;
        Ok(svc.list_running_sessions_for_user(user_id))
    }

    async fn stop_session(
        &self,
        _target_node_id: &str,
        session_id: &str,
    ) -> Result<StopSessionResponse, AppError> {
        let svc = self.get_service().await?;
        svc.stop_session(session_id)
            .await
            .map_err(|e| AppError::Internal(e))
    }

    async fn get_balance(
        &self,
        _target_node_id: &str,
        user_id: &str,
    ) -> Result<BalanceResponse, AppError> {
        let svc = self.get_service().await?;
        let balance = svc
            .get_user_balance(user_id)
            .await
            .map_err(|e| AppError::Internal(e))?;
        Ok(BalanceResponse {
            balance_sats: balance.max(0) as u64,
        })
    }

    async fn get_provider_cc_attestation(
        &self,
        _target_node_id: &str,
    ) -> Result<RawAttestationEvidence, AppError> {
        let svc = self.get_service().await?;
        svc.fetch_own_cc_attestation_raw()
            .await
            .map_err(AppError::Internal)
    }
}
