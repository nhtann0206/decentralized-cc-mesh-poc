//! M4: Bob's external workspace handlers — L402/Lightning protected.
//!
//! These routes are called by remote renters (Alice) via HttpWorkspaceClient.
//! Auth: Lightning signature (x-node-id + x-signature headers) verified by
//! L402 middleware. No JWT involved.
//!
//! Pattern: identical to HttpFriendServer::register_l402_routes.

use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio_tungstenite::connect_async;
use tracing::{debug, error, info};

use crate::dto::ApiResponse;
use crate::error::AppError;
use crate::extractors::NodeId;
use crate::state::AppState;
use crate::workspace::models::{
    RawAttestationEvidence, SecureComputer, StartSessionRequest, StartSessionResponse,
    StopSessionResponse, WorkspaceSession,
};

pub struct HttpWorkspaceExternalServer;

impl HttpWorkspaceExternalServer {
    /// Register L402-protected external workspace routes.
    /// Called from HttpServer::register_l402_routes.
    pub fn register_l402_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
        router
            .route(
                "/external/workspace/computers",
                get(Self::http_list_computers),
            )
            .route(
                "/external/workspace/sessions",
                post(Self::http_start_session),
            )
            .route(
                "/external/workspace/sessions/{session_id}",
                get(Self::http_get_session),
            )
            .route(
                "/external/workspace/sessions/{session_id}/stop",
                post(Self::http_stop_session),
            )
            .route(
                "/external/workspace/sessions/{session_id}/vnc",
                get(Self::http_vnc_proxy),
            )
            // Phase A — pre-rent CC proof. Alice (renter) asks Bob (provider)
            // for verifiable evidence that Bob's host is running on AMD SEV-SNP
            // BEFORE committing to rent. Bob returns its own host attestation;
            // any tenant Bob spawns inherits the same CC platform. This is the
            // proof step in the 4-step rent wizard: (1) browse, (2) verify CC,
            // (3) confirm + price, (4) start session.
            .route(
                "/external/workspace/cc-attestation",
                get(Self::http_provider_cc_attestation),
            )
    }

    /// Phase A — return Bob's OWN host SEV-SNP attestation as RAW bytes so
    /// the remote renter can verify the AMD chain HERSELF (zero trust in
    /// this node). Provider-level (no tenant yet) because the call happens
    /// in step 2 of the rent wizard, before any session is started.
    async fn http_provider_cc_attestation(
        State(state): State<Arc<AppState>>,
        NodeId(renter_node_id): NodeId,
    ) -> Result<Json<ApiResponse<RawAttestationEvidence>>, AppError> {
        debug!(
            target: "node_backend::workspace::external",
            renter_node_id = %renter_node_id,
            "Provider CC attestation requested"
        );

        let svc_guard = state.workspace_service.read().await;
        let svc = svc_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))?;

        let raw = svc
            .fetch_own_cc_attestation_raw()
            .await
            .map_err(AppError::Internal)?;

        Ok(Json(ApiResponse::success(
            raw,
            "Provider CC raw attestation evidence",
        )))
    }

    /// List available VMs for rent. Marketplace browsing.
    /// List THIS node's VM capacity for external renters.
    /// Uses list_local_computers() (not list_computers() which lists peers).
    async fn http_list_computers(
        State(state): State<Arc<AppState>>,
    ) -> Result<Json<ApiResponse<Vec<SecureComputer>>>, AppError> {
        let svc_guard = state.workspace_service.read().await;
        let svc = svc_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))?;

        let computers = svc.list_local_computers().await;
        Ok(Json(ApiResponse::success(computers, "Computers listed")))
    }

    /// Start a new VM rental session for a remote renter.
    /// The renter's identity comes from the x-node-id header (Lightning auth).
    async fn http_start_session(
        State(state): State<Arc<AppState>>,
        NodeId(renter_node_id): NodeId,
        Json(request): Json<StartSessionRequest>,
    ) -> Result<Json<ApiResponse<StartSessionResponse>>, AppError> {
        info!(
            target: "node_backend::workspace::external",
            renter_node_id = %renter_node_id,
            computer_id = %request.computer_id,
            "External workspace session request"
        );

        let svc_guard = state.workspace_service.read().await;
        let svc = svc_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))?;

        // Use renter's Lightning node_id as the user_id for this session.
        // This is the cross-node identity: each renter is identified by
        // their Lightning pubkey on Bob's node.
        let (session_id, status) = svc
            .start_session(&renter_node_id, request, None)
            .await
            .map_err(|e| AppError::Internal(e))?;

        Ok(Json(ApiResponse::success(
            StartSessionResponse {
                session_id,
                status,
            },
            "Session started",
        )))
    }

    /// Get session status for a remote renter.
    async fn http_get_session(
        State(state): State<Arc<AppState>>,
        Path(session_id): Path<String>,
    ) -> Result<Json<ApiResponse<WorkspaceSession>>, AppError> {
        let svc_guard = state.workspace_service.read().await;
        let svc = svc_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))?;

        let session = svc
            .get_session(&session_id)
            .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?;

        Ok(Json(ApiResponse::success(session, "Session retrieved")))
    }

    /// Stop a remote renter's session.
    async fn http_stop_session(
        State(state): State<Arc<AppState>>,
        NodeId(renter_node_id): NodeId,
        Path(session_id): Path<String>,
    ) -> Result<Json<ApiResponse<StopSessionResponse>>, AppError> {
        info!(
            target: "node_backend::workspace::external",
            renter_node_id = %renter_node_id,
            session_id = %session_id,
            "External workspace session stop request"
        );

        let svc = state
            .workspace_service
            .read()
            .await
            .clone()
            .ok_or_else(|| AppError::Internal("Workspace service not initialized".into()))?;

        let resp = svc
            .stop_session(&session_id)
            .await
            .map_err(|e| AppError::Internal(e))?;

        Ok(Json(ApiResponse::success(resp, "Session stopped")))
    }

    /// WebSocket proxy: connect Alice's noVNC client to QEMU's built-in
    /// VNC WebSocket. Bidirectional binary frame forwarding.
    ///
    /// Flow: Alice browser (noVNC) → Alice WS proxy → THIS handler →
    /// QEMU VNC WebSocket (localhost:590X) → VM framebuffer.
    async fn http_vnc_proxy(
        State(state): State<Arc<AppState>>,
        Path(session_id): Path<String>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        // noVNC hardcodes `Sec-WebSocket-Protocol: binary, base64` on the
        // browser side. Echo back `binary` so the WS handshake completes
        // per RFC 6455 §4.2.2.
        ws.protocols(["binary"])
            .on_upgrade(move |socket| Self::handle_vnc_proxy(state, session_id, socket))
    }

    async fn handle_vnc_proxy(
        state: Arc<AppState>,
        session_id: String,
        alice_ws: WebSocket,
    ) {
        // Look up session → get container_id
        let svc_guard = state.workspace_service.read().await;
        let Some(svc) = svc_guard.as_ref() else {
            error!(target: "node_backend::workspace::vnc", "Workspace service not initialized");
            return;
        };
        let Some(session) = svc.get_session(&session_id) else {
            error!(target: "node_backend::workspace::vnc", session_id = %session_id, "Session not found");
            return;
        };
        let Some(container_id) = session.container_id.as_deref() else {
            error!(target: "node_backend::workspace::vnc", session_id = %session_id, "No container_id");
            return;
        };

        // Resolve VNC WebSocket endpoint via runtime (host + port).
        // libvirt → ("127.0.0.1", 590X), GCP → (tenant_internal_ip, 6080).
        let Some((host, port)) = svc.get_vnc_endpoint(container_id).await else {
            error!(
                target: "node_backend::workspace::vnc",
                session_id = %session_id,
                container_id = %container_id,
                "VNC WebSocket endpoint not available"
            );
            return;
        };

        // Connect to upstream VNC WebSocket with `binary` subprotocol so the
        // handshake matches what noVNC expects (QEMU 5.0+ tolerant, websockify
        // also tolerant, but forwarding subprotocol keeps the chain consistent).
        let vnc_url = format!("ws://{}:{}", host, port);
        debug!(
            target: "node_backend::workspace::vnc",
            session_id = %session_id,
            vnc_url = %vnc_url,
            "Connecting to QEMU VNC WebSocket"
        );

        let qemu_req = match vnc_url.parse::<tokio_tungstenite::tungstenite::http::Uri>() {
            Ok(uri) => tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
                .with_sub_protocol("binary"),
            Err(e) => {
                error!(
                    target: "node_backend::workspace::vnc",
                    error = %e,
                    vnc_url = %vnc_url,
                    "Invalid QEMU VNC URL"
                );
                return;
            }
        };

        let qemu_ws = match connect_async(qemu_req).await {
            Ok((stream, _)) => stream,
            Err(e) => {
                error!(
                    target: "node_backend::workspace::vnc",
                    error = %e,
                    vnc_url = %vnc_url,
                    "Failed to connect to QEMU VNC"
                );
                return;
            }
        };

        info!(
            target: "node_backend::workspace::vnc",
            session_id = %session_id,
            "VNC proxy connected — forwarding frames"
        );

        // Split both WebSocket connections
        let (mut alice_sink, mut alice_stream) = alice_ws.split();
        let (mut qemu_sink, mut qemu_stream) = qemu_ws.split();

        // Forward: Alice → QEMU (keyboard/mouse input)
        let alice_to_qemu = tokio::spawn(async move {
            while let Some(Ok(msg)) = alice_stream.next().await {
                let tung_msg = match msg {
                    AxumWsMessage::Binary(data) => {
                        tokio_tungstenite::tungstenite::Message::Binary(data)
                    }
                    AxumWsMessage::Text(text) => {
                        tokio_tungstenite::tungstenite::Message::text(text.to_string())
                    }
                    AxumWsMessage::Ping(data) => {
                        tokio_tungstenite::tungstenite::Message::Ping(data)
                    }
                    AxumWsMessage::Close(_) => break,
                    _ => continue,
                };
                if qemu_sink.send(tung_msg).await.is_err() {
                    break;
                }
            }
        });

        // Forward: QEMU → Alice (framebuffer updates)
        let qemu_to_alice = tokio::spawn(async move {
            while let Some(Ok(msg)) = qemu_stream.next().await {
                let axum_msg = match msg {
                    tokio_tungstenite::tungstenite::Message::Binary(data) => {
                        AxumWsMessage::Binary(data)
                    }
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        AxumWsMessage::Text(text.to_string().into())
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(data) => {
                        AxumWsMessage::Ping(data)
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    _ => continue,
                };
                if alice_sink.send(axum_msg).await.is_err() {
                    break;
                }
            }
        });

        // Wait for either direction to close
        tokio::select! {
            _ = alice_to_qemu => {},
            _ = qemu_to_alice => {},
        }

        debug!(
            target: "node_backend::workspace::vnc",
            session_id = %session_id,
            "VNC proxy disconnected"
        );
    }
}
