//! HttpWorkspaceClient — HTTP client for cross-node VM rental.
//!
//! Implements `PublicWorkspaceEndpoint` as a CLIENT — Alice calls this
//! to rent VMs from Bob. Uses L402HttpClient for automatic Lightning
//! payment + node authentication (x-node-id + x-signature headers).
//!
//! Pattern: identical to HttpFriendClient. L402HttpClient resolves
//! target_node_id → URL via IP pool, handles 402 auto-pay, adds
//! Lightning auth headers. No JWT involved.

use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::dto::ApiResponse;
use crate::endpoints::PublicWorkspaceEndpoint;
use crate::error::AppError;
use crate::l402::client::L402HttpClient;
use crate::workspace::models::{
    BalanceResponse, RawAttestationEvidence, SecureComputer, StartSessionRequest,
    StartSessionResponse, StopSessionResponse, WorkspaceSession,
};

pub struct HttpWorkspaceClient {
    l402_client: L402HttpClient,
}

impl HttpWorkspaceClient {
    pub fn new(l402_client: L402HttpClient) -> Self {
        Self { l402_client }
    }
}

#[async_trait]
impl PublicWorkspaceEndpoint for HttpWorkspaceClient {
    async fn list_computers(
        &self,
        target_node_id: &str,
    ) -> Result<Vec<SecureComputer>, AppError> {
        debug!(
            target: "node_backend::node_client::workspace",
            target_node = %target_node_id,
            "Fetching marketplace from remote node"
        );

        let response = self
            .l402_client
            .get(target_node_id, "api/v2/external/workspace/computers")
            .await
            .map_err(|e| {
                warn!(
                    target: "node_backend::node_client::workspace",
                    error = %e,
                    target_node = %target_node_id,
                    "Failed to fetch peer computers"
                );
                AppError::External(format!("Request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Peer returned {}: {}", status, body)));
        }

        let api_resp: ApiResponse<Vec<SecureComputer>> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        api_resp
            .data
            .ok_or_else(|| AppError::External("Peer returned success but no data".into()))
    }

    async fn start_session(
        &self,
        target_node_id: &str,
        user_id: &str,
        request: StartSessionRequest,
    ) -> Result<StartSessionResponse, AppError> {
        info!(
            target: "node_backend::node_client::workspace",
            target_node = %target_node_id,
            computer_id = %request.computer_id,
            "Starting remote workspace session"
        );

        let builder = self
            .l402_client
            .post(target_node_id, "api/v2/external/workspace/sessions")
            .await
            .map_err(|e| AppError::External(format!("Resolve endpoint: {}", e)))?;

        let response = builder.json(&request).send().await.map_err(|e| {
            warn!(
                target: "node_backend::node_client::workspace",
                error = %e,
                "Failed to start remote session"
            );
            AppError::External(format!("Request failed: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Peer returned {}: {}", status, body)));
        }

        let api_resp: ApiResponse<StartSessionResponse> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;

        let session = api_resp
            .data
            .ok_or_else(|| AppError::External("No data in response".into()))?;

        info!(
            target: "node_backend::node_client::workspace",
            session_id = %session.session_id,
            "Remote session started"
        );
        Ok(session)
    }

    async fn get_session(
        &self,
        target_node_id: &str,
        session_id: &str,
    ) -> Result<Option<WorkspaceSession>, AppError> {
        let response = self
            .l402_client
            .get(
                target_node_id,
                &format!("api/v2/external/workspace/sessions/{}", session_id),
            )
            .await
            .map_err(|e| AppError::External(format!("Request failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Peer returned {}: {}", status, body)));
        }

        let api_resp: ApiResponse<WorkspaceSession> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        Ok(api_resp.data)
    }

    async fn list_user_sessions(
        &self,
        target_node_id: &str,
        _user_id: &str,
    ) -> Result<Vec<WorkspaceSession>, AppError> {
        let response = self
            .l402_client
            .get(target_node_id, "api/v2/external/workspace/sessions")
            .await
            .map_err(|e| AppError::External(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let api_resp: ApiResponse<Vec<WorkspaceSession>> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        Ok(api_resp.data.unwrap_or_default())
    }

    async fn stop_session(
        &self,
        target_node_id: &str,
        session_id: &str,
    ) -> Result<StopSessionResponse, AppError> {
        info!(
            target: "node_backend::node_client::workspace",
            target_node = %target_node_id,
            session_id = %session_id,
            "Stopping remote session"
        );

        let builder = self
            .l402_client
            .post(
                target_node_id,
                &format!("api/v2/external/workspace/sessions/{}/stop", session_id),
            )
            .await
            .map_err(|e| AppError::External(format!("Resolve endpoint: {}", e)))?;

        let response = builder
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| {
                AppError::External(format!("Request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Peer returned {}: {}", status, body)));
        }

        let api_resp: ApiResponse<StopSessionResponse> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        api_resp
            .data
            .ok_or_else(|| AppError::External("No data in response".into()))
    }

    async fn get_balance(
        &self,
        target_node_id: &str,
        _user_id: &str,
    ) -> Result<BalanceResponse, AppError> {
        let response = self
            .l402_client
            .get(target_node_id, "api/v2/external/workspace/balance")
            .await
            .map_err(|e| AppError::External(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(BalanceResponse { balance_sats: 0 });
        }

        let api_resp: ApiResponse<BalanceResponse> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        Ok(api_resp.data.unwrap_or(BalanceResponse { balance_sats: 0 }))
    }

    async fn get_provider_cc_attestation(
        &self,
        target_node_id: &str,
    ) -> Result<RawAttestationEvidence, AppError> {
        debug!(
            target: "node_backend::node_client::workspace",
            target_node = %target_node_id,
            "Fetching provider CC attestation evidence"
        );

        let response = self
            .l402_client
            .get(target_node_id, "api/v2/external/workspace/cc-attestation")
            .await
            .map_err(|e| AppError::External(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Peer returned {}: {}", status, body)));
        }

        let api_resp: ApiResponse<RawAttestationEvidence> = response.json().await.map_err(|e| {
            AppError::External(format!("Parse response: {}", e))
        })?;
        api_resp
            .data
            .ok_or_else(|| AppError::External("Peer returned success but no data".into()))
    }
}
