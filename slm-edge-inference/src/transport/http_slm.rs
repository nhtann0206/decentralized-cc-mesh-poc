//! HttpSlmServer - HTTP transport layer for SLM Edge Inference
//!
//! Provides HTTP handlers for:
//! - SLM subsystem status (JWT-protected)
//! - Model listing (JWT-protected)
//! - Model pull requests (JWT-protected)
//!
//! Part of feature 040: SLM Edge Inference & Model Routing.
//!
//! ## Route Structure
//!
//! JWT-Protected (v2 API):
//! - GET  /api/v2/slm/status - SLM subsystem status (memory, models, routing)
//! - GET  /api/v2/slm/models - List available models
//! - POST /api/v2/slm/models/pull - Pull a model from registry

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;
use tracing::info;

use crate::dto::ApiResponse;
use crate::error::AppError;
use crate::slm::models::{ModelInfo, PullModelRequest, SlmStatus};
use crate::state::AppState;

/// Response for model pull operation.
#[derive(Debug, Serialize)]
pub struct PullModelResponse {
    pub model: String,
    pub status: String,
}

pub struct HttpSlmServer;

impl HttpSlmServer {
    /// Register JWT-protected SLM routes under /api/v2/slm/
    pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
        router
            .route("/slm/status", get(Self::http_status))
            .route("/slm/models", get(Self::http_list_models))
            .route("/slm/models/pull", post(Self::http_pull_model))
    }

    /// GET /api/v2/slm/status - SLM subsystem status
    async fn http_status(
        State(state): State<Arc<AppState>>,
    ) -> Result<Json<ApiResponse<SlmStatus>>, AppError> {
        let (router, lifecycle) = match (&state.slm_router, &state.slm_lifecycle) {
            (Some(r), Some(l)) => (r, l),
            _ => {
                // SLM disabled — return minimal status
                let status = SlmStatus {
                    enabled: false,
                    provider: String::new(),
                    ollama_available: false,
                    ollama_url: String::new(),
                    memory: crate::slm::MemoryBudget {
                        used_mb: 0,
                        limit_mb: 0,
                        available_mb: 0,
                        loaded_models: vec![],
                    },
                    models: vec![],
                    default_model: None,
                };
                return Ok(Json(ApiResponse::success(status, "SLM disabled")));
            }
        };

        let ollama_available = router.local_provider().is_available().await;
        let models = lifecycle.list_models().await.unwrap_or_default();
        let running = lifecycle.list_running().await.unwrap_or_default();
        let loaded_names: Vec<String> = running.iter().map(|m| m.name.clone()).collect();

        let memory = router.memory_manager().budget();
        let memory_with_models = crate::slm::MemoryBudget {
            loaded_models: loaded_names,
            ..memory
        };

        // Update Prometheus gauges
        crate::slm::metrics::update_memory_gauges(memory_with_models.used_mb, memory_with_models.limit_mb);
        crate::slm::metrics::SLM_OLLAMA_AVAILABLE.set(if ollama_available { 1 } else { 0 });
        crate::slm::metrics::SLM_MODELS_LOADED.set(running.len() as i64);

        let status = SlmStatus {
            enabled: true,
            provider: router.local_provider().provider_name().to_string(),
            ollama_available,
            ollama_url: router.config().ollama_url.clone(),
            memory: memory_with_models,
            models,
            default_model: router.config().default_model.clone(),
        };

        Ok(Json(ApiResponse::success(status, "SLM status")))
    }

    /// GET /api/v2/slm/models - List available models
    async fn http_list_models(
        State(state): State<Arc<AppState>>,
    ) -> Result<Json<ApiResponse<Vec<ModelInfo>>>, AppError> {
        let lifecycle = state.slm_lifecycle.as_ref().ok_or_else(|| {
            AppError::BadRequest("SLM not enabled on this node".to_string())
        })?;

        let models = lifecycle.list_models().await?;

        Ok(Json(ApiResponse::success(models, "Models listed")))
    }

    /// POST /api/v2/slm/models/pull - Pull a model from registry
    async fn http_pull_model(
        State(state): State<Arc<AppState>>,
        Json(request): Json<PullModelRequest>,
    ) -> Result<Json<ApiResponse<PullModelResponse>>, AppError> {
        let lifecycle = state.slm_lifecycle.as_ref().ok_or_else(|| {
            AppError::BadRequest("SLM not enabled on this node".to_string())
        })?;

        info!(
            target: "node_backend::slm",
            model = %request.model,
            "Pulling model via API"
        );

        lifecycle.pull_model(&request.model).await?;

        let response = PullModelResponse {
            model: request.model,
            status: "pulled".to_string(),
        };

        Ok(Json(ApiResponse::success(response, "Model pulled successfully")))
    }
}
