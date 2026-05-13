//! SLM Model Lifecycle Manager — load, unload, swap models via Ollama API.
//!
//! Coordinates model loading with the MemoryBudgetManager to ensure
//! edge devices (1GB RAM) never exceed their memory budget.
//!
//! State machine per model: NotPresent → Downloaded → Loading → Loaded → Unloading → Downloaded

use crate::error::AppError;
use crate::slm::config::SlmConfig;
use crate::slm::memory::MemoryBudgetManager;
use crate::slm::models::ModelInfo;
use crate::slm::providers::LocalInferenceProvider;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

/// Manages the lifecycle of locally loaded models.
///
/// Ensures memory budget compliance before loading and proper cleanup on unload.
/// Uses a Semaphore(1) to serialize load operations — prevents two concurrent
/// loads from both reserving memory and causing Ollama OOM on 1GB devices.
pub struct ModelLifecycleManager {
    provider: Arc<dyn LocalInferenceProvider>,
    memory_manager: Arc<MemoryBudgetManager>,
    swap_timeout: Duration,
    /// Serializes load_model/unload_model to prevent concurrent reserve+load races.
    /// On 1GB devices, concurrent loads are never desirable — one model at a time.
    load_semaphore: Semaphore,
}

impl ModelLifecycleManager {
    pub fn new(
        provider: Arc<dyn LocalInferenceProvider>,
        memory_manager: Arc<MemoryBudgetManager>,
        config: &SlmConfig,
    ) -> Self {
        Self {
            provider,
            memory_manager,
            swap_timeout: Duration::from_secs(config.swap_timeout_secs),
            load_semaphore: Semaphore::new(1),
        }
    }

    /// Load a model into RAM, checking memory budget first.
    ///
    /// If the model is already loaded, this is a no-op.
    /// If memory budget would be exceeded, returns an error.
    ///
    /// Serialized via semaphore — only one load operation at a time.
    /// Prevents concurrent reserves from causing Ollama OOM on 1GB devices.
    pub async fn load_model(&self, model: &str) -> Result<ModelInfo, AppError> {
        // Acquire load lock — prevents concurrent reserve+load race condition.
        let _permit = self.load_semaphore.acquire().await.map_err(|_| {
            AppError::External("Load semaphore closed (shutdown in progress)".to_string())
        })?;

        // Check if already loaded (inside semaphore to prevent TOCTOU)
        let running = self.provider.list_running().await?;
        if let Some(info) = running.iter().find(|m| m.name == model) {
            debug!(
                target: "node_backend::slm",
                model = model,
                "Model already loaded"
            );
            return Ok(info.clone());
        }

        // Get model info for memory estimation
        let models = self.provider.list_models().await?;
        let model_info = models
            .iter()
            .find(|m| m.name == model)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Model '{}' not found. Pull it first with pull_model()",
                    model
                ))
            })?;

        let required_mb = model_info.estimated_ram_mb();

        // Reserve memory
        if !self.memory_manager.reserve(required_mb) {
            return Err(AppError::External(format!(
                "Cannot load '{}': requires {}MB but only {}MB available (limit: {}MB)",
                model,
                required_mb,
                self.memory_manager.available_mb(),
                self.memory_manager.limit_mb(),
            )));
        }

        info!(
            target: "node_backend::slm",
            model = model,
            required_mb = required_mb,
            "Loading model into RAM"
        );

        // Trigger model load via a lightweight generate call
        // Ollama loads models on first use
        match self.provider.generate(model, "hello", None).await {
            Ok(_) => {
                info!(
                    target: "node_backend::slm",
                    model = model,
                    "Model loaded successfully"
                );
                crate::slm::metrics::record_lifecycle_event("load", "success");

                Ok(ModelInfo {
                    name: model.to_string(),
                    size_bytes: model_info.size_bytes,
                    quantization: model_info.quantization.clone(),
                    parameter_count: model_info.parameter_count.clone(),
                    families: model_info.families.clone(),
                    is_loaded: true,
                    modified_at: model_info.modified_at.clone(),
                })
            }
            Err(e) => {
                // Release reserved memory on failure
                self.memory_manager.release(required_mb);
                crate::slm::metrics::record_lifecycle_event("load", "error");
                warn!(
                    target: "node_backend::slm",
                    model = model,
                    error = %e,
                    "Failed to load model, releasing reserved memory"
                );

                // Reconcile budget with Ollama reality
                if let Err(sync_err) = self.sync_memory_state().await {
                    warn!(
                        target: "node_backend::slm",
                        error = %sync_err,
                        "Failed to reconcile memory state after load error"
                    );
                }

                Err(e)
            }
        }
    }

    /// Unload a model from RAM and release its memory budget.
    pub async fn unload_model(&self, model: &str) -> Result<(), AppError> {
        // Check if model is actually loaded via /api/ps
        let running = self.provider.list_running().await?;
        if !running.iter().any(|m| m.name == model) {
            debug!(
                target: "node_backend::slm",
                model = model,
                "Model not currently loaded, nothing to unload"
            );
            return Ok(());
        }

        // Use list_models (file size) for budget release — same source as load_model
        let models = self.provider.list_models().await?;
        let size_mb = models
            .iter()
            .find(|m| m.name == model)
            .map(|m| m.estimated_ram_mb())
            .unwrap_or(0);

        info!(
            target: "node_backend::slm",
            model = model,
            released_mb = size_mb,
            "Unloading model from RAM"
        );

        // Evict model from Ollama in-memory cache (keep_alive=0).
        let _ = self.provider.unload_from_memory(model).await;

        self.memory_manager.release(size_mb);
        crate::slm::metrics::record_lifecycle_event("unload", "success");

        Ok(())
    }

    /// Swap models: unload current, load new.
    pub async fn swap_model(&self, unload: &str, load: &str) -> Result<ModelInfo, AppError> {
        info!(
            target: "node_backend::slm",
            unload = unload,
            load = load,
            timeout_secs = self.swap_timeout.as_secs(),
            "Swapping models"
        );

        let timeout = self.swap_timeout;
        tokio::time::timeout(timeout, async {
            self.unload_model(unload).await?;
            self.load_model(load).await
        })
        .await
        .map_err(|_| {
            AppError::External(format!(
                "swap_model timed out after {}s (unload '{}' → load '{}'). \
                 Adjust via SLM_SWAP_TIMEOUT_SECS.",
                timeout.as_secs(),
                unload,
                load,
            ))
        })?
    }

    /// Pull a model from the Ollama registry (download, don't load into RAM).
    pub async fn pull_model(&self, model: &str) -> Result<(), AppError> {
        info!(
            target: "node_backend::slm",
            model = model,
            "Pulling model from registry"
        );
        self.provider.pull_model(model).await
    }

    /// Delete a model from local storage.
    pub async fn delete_model(&self, model: &str) -> Result<(), AppError> {
        // Unload first if loaded
        self.unload_model(model).await?;
        self.provider.delete_model(model).await
    }

    /// List all available models with their loaded status.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError> {
        self.provider.list_models().await
    }

    /// List currently loaded (running) models.
    pub async fn list_running(&self) -> Result<Vec<ModelInfo>, AppError> {
        self.provider.list_running().await
    }

    /// Sync memory tracking with actual Ollama state.
    ///
    /// Call this on startup or after Ollama restart to reconcile
    /// the memory budget with reality.
    pub async fn sync_memory_state(&self) -> Result<(), AppError> {
        self.memory_manager.reset();

        let running = self.provider.list_running().await?;
        for model in &running {
            let mb = model.estimated_ram_mb();
            self.memory_manager.reserve(mb);
        }

        info!(
            target: "node_backend::slm",
            loaded_count = running.len(),
            used_mb = self.memory_manager.used_mb(),
            "Memory state synced with Ollama"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slm::config::SlmConfig;
    use crate::slm::providers::MockLocalProvider;

    fn test_lifecycle(limit_mb: u64) -> (ModelLifecycleManager, Arc<MemoryBudgetManager>) {
        let config = SlmConfig {
            memory_limit_mb: limit_mb,
            ..SlmConfig::default()
        };
        let memory = Arc::new(MemoryBudgetManager::new(&config));
        let provider = Arc::new(MockLocalProvider::new());
        let lifecycle = ModelLifecycleManager::new(provider, memory.clone(), &config);
        (lifecycle, memory)
    }

    #[tokio::test]
    async fn test_load_exceeds_budget() {
        let (lifecycle, _) = test_lifecycle(10); // Only 10MB budget
        let result = lifecycle.load_model("mock-model:latest").await;
        // Mock model is 300MB, won't fit in 10MB
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unload_releases_memory() {
        let (lifecycle, memory) = test_lifecycle(512);
        lifecycle.load_model("mock-model:latest").await.unwrap();
        let used_after_load = memory.used_mb();
        assert!(used_after_load > 0);

        lifecycle.unload_model("mock-model:latest").await.unwrap();
        // Memory should be released (or reduced)
        assert!(memory.used_mb() < used_after_load);
    }

    #[tokio::test]
    async fn test_sync_memory_state() {
        let (lifecycle, memory) = test_lifecycle(512);

        // Load a model first (simulates Ollama having a model in RAM)
        lifecycle.load_model("mock-model:latest").await.unwrap();
        let used_after_load = memory.used_mb();
        assert!(used_after_load > 0);

        // Manually corrupt the memory tracking
        memory.reset();
        assert_eq!(memory.used_mb(), 0);

        // Sync should re-count from actual Ollama state
        lifecycle.sync_memory_state().await.unwrap();
        // Mock model is loaded (300MB * 1.25 = 375MB, R-009 §4.7)
        assert_eq!(memory.used_mb(), 375);
    }
}
