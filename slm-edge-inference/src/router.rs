//! SLM Model Router — LOCAL decision engine.
//!
//! Determines whether THIS node can handle inference locally (memory budget,
//! model availability, context window). Cloud inference routes through
//! CapabilityRouter → API Store builtin app.
//!
//! - `LocalOnly`  → require local or return Err
//! - `CloudOnly`  → route to cloud API (CapabilityRouter → API Store)
//!
//! Routing is pure logic (~O(1)) — no I/O in the decision path itself.

use crate::error::AppError;
use crate::slm::config::SlmConfig;
use crate::slm::memory::MemoryBudgetManager;
use crate::slm::models::{InferenceTarget, RoutingDecision};
use crate::slm::providers::LocalInferenceProvider;
use std::sync::Arc;
use tracing::debug;

/// Routing preference for a specific request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingPreference {
    /// Only use local SLM — error if unavailable.
    LocalOnly,
    /// Only use cloud API — skip local entirely.
    CloudOnly,
}

impl Default for RoutingPreference {
    fn default() -> Self {
        Self::LocalOnly
    }
}

/// Request context for routing decisions.
#[derive(Debug, Clone)]
pub struct RoutingRequest {
    /// The model explicitly requested (if any).
    pub model: Option<String>,
    /// Routing preference — caller must choose Local or Cloud explicitly.
    pub preference: RoutingPreference,
    /// Estimated prompt token count (for capability matching).
    pub estimated_tokens: Option<u32>,
}

impl Default for RoutingRequest {
    fn default() -> Self {
        Self {
            model: None,
            preference: RoutingPreference::default(),
            estimated_tokens: None,
        }
    }
}

/// Model routing decision engine.
///
/// Examines local provider availability, memory budget, and user preferences
/// to select the inference target. No automatic fallback — caller receives
/// an explicit error when the requested target is unavailable.
pub struct ModelRouter {
    config: SlmConfig,
    memory_manager: Arc<MemoryBudgetManager>,
    local_provider: Arc<dyn LocalInferenceProvider>,
}

impl ModelRouter {
    pub fn new(
        config: SlmConfig,
        memory_manager: Arc<MemoryBudgetManager>,
        local_provider: Arc<dyn LocalInferenceProvider>,
    ) -> Self {
        Self {
            config,
            memory_manager,
            local_provider,
        }
    }

    /// Make a routing decision for the given request.
    ///
    /// - `LocalOnly` → require local or return Err (no auto-fallback to cloud)
    /// - `CloudOnly` → route to cloud API directly
    pub async fn route(&self, request: &RoutingRequest) -> Result<RoutingDecision, AppError> {
        debug!(
            target: "node_backend::slm",
            preference = ?request.preference,
            model = ?request.model,
            "Routing inference request"
        );

        match &request.preference {
            RoutingPreference::LocalOnly => self.route_local_only(request).await,
            RoutingPreference::CloudOnly => Ok(self.route_to_cloud()),
        }
    }

    /// Route exclusively to local SLM. Error if unavailable or over-budget.
    async fn route_local_only(&self, request: &RoutingRequest) -> Result<RoutingDecision, AppError> {
        match self.check_local_availability(request).await {
            Some(decision) => Ok(decision),
            None => Err(AppError::External(format!(
                "Local SLM unavailable ({}) — use CloudOnly or ensure the inference runtime is running",
                self.local_provider.provider_name()
            ))),
        }
    }

    /// Check if local inference is possible and return a routing decision.
    async fn check_local_availability(&self, request: &RoutingRequest) -> Option<RoutingDecision> {
        // 1. Is the local inference runtime running?
        if !self.local_provider.is_available().await {
            debug!(
                target: "node_backend::slm",
                provider = self.local_provider.provider_name(),
                "Local inference provider not available"
            );
            return None;
        }

        // 2. Is the requested model available?
        let model = request
            .model
            .clone()
            .or_else(|| self.config.default_model.clone())
            .unwrap_or_else(|| crate::slm::config::DEFAULT_MODEL.to_string());

        let models = self.local_provider.list_models().await.ok()?;
        let model_info = models.iter().find(|m| m.name == model);

        match model_info {
            Some(info) => {
                // 3. Check memory budget if not already loaded
                if !info.is_loaded {
                    let required_mb = info.estimated_ram_mb();
                    if !self.memory_manager.can_load(required_mb) {
                        debug!(
                            target: "node_backend::slm",
                            model = model,
                            required_mb = required_mb,
                            available_mb = self.memory_manager.available_mb(),
                            "Model exceeds memory budget"
                        );
                        return None;
                    }
                }

                // 4. Check context window
                if let Some(tokens) = request.estimated_tokens {
                    if tokens > self.config.max_context_tokens {
                        debug!(
                            target: "node_backend::slm",
                            tokens = tokens,
                            limit = self.config.max_context_tokens,
                            "Request exceeds context window"
                        );
                        return None;
                    }
                }

                Some(RoutingDecision {
                    target: InferenceTarget::Local {
                        provider: self.local_provider.provider_name().to_string(),
                        model,
                    },
                    reason: if info.is_loaded {
                        "Model loaded in RAM, ready for inference".to_string()
                    } else {
                        "Model available locally, within memory budget".to_string()
                    },
                    estimated_latency_ms: Some(if info.is_loaded { 500 } else { 5000 }),
                })
            }
            None => {
                debug!(
                    target: "node_backend::slm",
                    model = model,
                    "Model not found locally"
                );
                None
            }
        }
    }

    /// Create a routing decision for cloud inference.
    fn route_to_cloud(&self) -> RoutingDecision {
        RoutingDecision {
            target: InferenceTarget::Cloud {
                platform: "api_store".to_string(),
                model: "default".to_string(),
            },
            reason: "Routed to cloud API (CloudOnly preference)".to_string(),
            estimated_latency_ms: Some(200),
        }
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &SlmConfig {
        &self.config
    }

    /// Get a reference to the memory manager.
    pub fn memory_manager(&self) -> &Arc<MemoryBudgetManager> {
        &self.memory_manager
    }

    /// Get a reference to the local provider.
    pub fn local_provider(&self) -> &Arc<dyn LocalInferenceProvider> {
        &self.local_provider
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slm::providers::MockLocalProvider;

    fn test_router(provider: MockLocalProvider) -> ModelRouter {
        let config = SlmConfig {
            enabled: true,
            default_model: Some("mock-model:latest".to_string()),
            ..SlmConfig::default()
        };
        let memory = Arc::new(MemoryBudgetManager::new(&config));
        ModelRouter::new(config, memory, Arc::new(provider))
    }

    #[tokio::test]
    async fn test_route_local_available() {
        let router = test_router(MockLocalProvider::new());
        let request = RoutingRequest::default(); // LocalOnly

        let decision = router.route(&request).await.unwrap();
        assert!(decision.target.is_local());
    }

    #[tokio::test]
    async fn test_route_local_unavailable_errors() {
        // LocalOnly with no Ollama → explicit Err, no silent cloud fallback
        let router = test_router(MockLocalProvider::unavailable());
        let request = RoutingRequest::default(); // LocalOnly

        let result = router.route(&request).await;
        assert!(result.is_err(), "Expected Err when local unavailable with LocalOnly");
    }

    #[tokio::test]
    async fn test_route_cloud_only() {
        let router = test_router(MockLocalProvider::new());
        let request = RoutingRequest {
            preference: RoutingPreference::CloudOnly,
            ..Default::default()
        };

        let decision = router.route(&request).await.unwrap();
        assert!(decision.target.is_cloud());
    }

    #[tokio::test]
    async fn test_route_cloud_only_when_local_unavailable() {
        // CloudOnly always succeeds regardless of Ollama availability
        let router = test_router(MockLocalProvider::unavailable());
        let request = RoutingRequest {
            preference: RoutingPreference::CloudOnly,
            ..Default::default()
        };

        let decision = router.route(&request).await.unwrap();
        assert!(decision.target.is_cloud());
    }

    #[tokio::test]
    async fn test_route_local_model_not_found_errors() {
        let router = test_router(MockLocalProvider::new());
        let request = RoutingRequest {
            model: Some("nonexistent:latest".to_string()),
            ..Default::default() // LocalOnly
        };

        let result = router.route(&request).await;
        assert!(result.is_err(), "Model not found locally should Err with LocalOnly");
    }

    #[tokio::test]
    async fn test_route_local_context_exceeded_errors() {
        let router = test_router(MockLocalProvider::new());
        let request = RoutingRequest {
            estimated_tokens: Some(999_999), // Way over 2048
            ..Default::default() // LocalOnly
        };

        let result = router.route(&request).await;
        assert!(result.is_err(), "Context window exceeded should Err with LocalOnly");
    }

    #[tokio::test]
    async fn test_route_local_memory_exceeded_errors() {
        let config = SlmConfig {
            enabled: true,
            memory_limit_mb: 100,
            default_model: Some("mock-model:latest".to_string()),
            ..SlmConfig::default()
        };
        let memory = Arc::new(MemoryBudgetManager::new(&config));
        memory.reserve(99); // Only 1MB free, model needs ~300MB

        let provider = MockLocalProvider::new();
        let router = ModelRouter::new(config, memory, Arc::new(provider));

        let request = RoutingRequest::default(); // LocalOnly
        let result = router.route(&request).await;
        assert!(result.is_err(), "Memory exceeded should Err with LocalOnly");
    }
}
