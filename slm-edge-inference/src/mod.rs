//! SLM Edge Inference & Model Routing module (040-slm-edge-inference).
//!
//! Adds "SLMs for edge device" inference to Lightning Node OS — an abstract group
//! of local inference runtimes (Ollama, llama.cpp, MLX, etc.) behind the
//! `LocalInferenceProvider` trait. Explicit routing: `LocalOnly` or `CloudOnly`.
//!
//! Cloud inference routes through CapabilityRouter → API Store builtin app.
//! This module is the LOCAL decision engine only.
//!
//! Feature-gated: `SLM_ENABLED=false` (default) has zero impact on existing code.
//! Designed for resource-constrained edge devices (Radxa Zero 3W, 1GB RAM).

pub mod config;
pub mod lifecycle;
pub mod memory;
pub mod metrics;
pub mod models;
pub mod providers;
pub mod router;

pub use config::SlmConfig;
pub use lifecycle::ModelLifecycleManager;
pub use memory::MemoryBudgetManager;
pub use models::{InferenceTarget, MemoryBudget, ModelInfo, RoutingDecision, SlmStatus};
pub use router::{ModelRouter, RoutingPreference, RoutingRequest};

use providers::{MockLocalProvider, OllamaLocalProvider};
use std::sync::Arc;

/// Initialized SLM subsystem components.
///
/// Returned by `initialize_slm()` when SLM is enabled.
pub struct SlmSubsystem {
    pub router: Arc<ModelRouter>,
    pub lifecycle: Arc<ModelLifecycleManager>,
    pub memory_manager: Arc<MemoryBudgetManager>,
}

/// Initialize the SLM subsystem based on configuration.
///
/// Returns `None` if SLM is disabled (`SLM_ENABLED=false`).
/// Returns `Some(SlmSubsystem)` with all components wired together.
pub fn initialize_slm(config: &SlmConfig) -> Option<SlmSubsystem> {
    if !config.enabled {
        tracing::info!(
            target: "node_backend::slm",
            "SLM disabled, skipping initialization"
        );
        return None;
    }

    tracing::info!(
        target: "node_backend::slm",
        ollama_url = %config.ollama_url,
        memory_limit_mb = config.memory_limit_mb,
        default_model = ?config.default_model,
        "Initializing SLM subsystem"
    );

    let memory_manager = Arc::new(MemoryBudgetManager::new(config));

    // Update Prometheus gauges
    metrics::update_memory_gauges(0, config.memory_limit_mb);

    let provider: Arc<dyn providers::LocalInferenceProvider> =
        Arc::new(OllamaLocalProvider::new(&config.ollama_url, config.ollama_timeout_secs));

    let router = Arc::new(ModelRouter::new(
        config.clone(),
        memory_manager.clone(),
        provider.clone(),
    ));

    let lifecycle = Arc::new(ModelLifecycleManager::new(
        provider,
        memory_manager.clone(),
        config,
    ));

    Some(SlmSubsystem {
        router,
        lifecycle,
        memory_manager,
    })
}

/// Initialize SLM with mock provider for testing.
///
/// Returns a fully functional subsystem with `MockLocalProvider`
/// that doesn't require Ollama to be running.
pub fn initialize_slm_mock(config: &SlmConfig) -> SlmSubsystem {
    let memory_manager = Arc::new(MemoryBudgetManager::new(config));
    let provider: Arc<dyn providers::LocalInferenceProvider> =
        Arc::new(MockLocalProvider::new());

    let router = Arc::new(ModelRouter::new(
        config.clone(),
        memory_manager.clone(),
        provider.clone(),
    ));

    let lifecycle = Arc::new(ModelLifecycleManager::new(
        provider,
        memory_manager.clone(),
        config,
    ));

    SlmSubsystem {
        router,
        lifecycle,
        memory_manager,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_disabled() {
        let config = SlmConfig::default(); // enabled: false
        assert!(initialize_slm(&config).is_none());
    }

    #[tokio::test]
    async fn test_full_mock_flow() {
        let config = SlmConfig {
            enabled: true,
            default_model: Some("mock-model:latest".to_string()),
            ..SlmConfig::default()
        };
        let subsystem = initialize_slm_mock(&config);

        // Route should succeed with mock
        let request = RoutingRequest::default();
        let decision = subsystem.router.route(&request).await.unwrap();
        assert!(decision.target.is_local());

        // List models should work
        let models = subsystem.lifecycle.list_models().await.unwrap();
        assert!(!models.is_empty());

        // Memory budget should be tracked
        let budget = subsystem.memory_manager.budget();
        assert_eq!(budget.limit_mb, 384);
    }
}
