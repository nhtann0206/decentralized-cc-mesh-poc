//! Mock local inference provider for testing without Ollama.
//!
//! Returns configurable responses, simulates model loading/unloading,
//! and tracks calls for assertion in tests.

use crate::error::AppError;
use crate::slm::models::ModelInfo;
use crate::slm::providers::{ChatMessage, ChatResponse, LocalInferenceProvider};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Mock provider for unit/integration tests.
///
/// Configurable availability and responses. No external dependencies.
pub struct MockLocalProvider {
    available: AtomicBool,
    models: Mutex<Vec<ModelInfo>>,
    response_text: Mutex<String>,
}

impl MockLocalProvider {
    pub fn new() -> Self {
        Self {
            available: AtomicBool::new(true),
            models: Mutex::new(vec![ModelInfo {
                name: "mock-model:latest".to_string(),
                size_bytes: 300 * 1024 * 1024,
                quantization: Some("Q4_K_M".to_string()),
                parameter_count: Some("0.5B".to_string()),
                families: vec!["mock".to_string()],
                is_loaded: false,
                modified_at: None,
            }]),
            response_text: Mutex::new("Mock SLM response".to_string()),
        }
    }

    /// Create a mock provider that reports as unavailable.
    pub fn unavailable() -> Self {
        let provider = Self::new();
        provider.available.store(false, Ordering::Relaxed);
        provider
    }

    /// Set the response text returned by generate/chat.
    pub fn set_response(&self, text: &str) {
        *self.response_text.lock().unwrap() = text.to_string();
    }

    /// Set whether the provider reports as available.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::Relaxed);
    }

    /// Add a model to the mock's model list.
    pub fn add_model(&self, model: ModelInfo) {
        self.models.lock().unwrap().push(model);
    }
}

impl Default for MockLocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LocalInferenceProvider for MockLocalProvider {
    fn provider_name(&self) -> &'static str {
        "mock"
    }

    async fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError> {
        Ok(self.models.lock().unwrap().clone())
    }

    async fn list_running(&self) -> Result<Vec<ModelInfo>, AppError> {
        let models = self.models.lock().unwrap();
        Ok(models.iter().filter(|m| m.is_loaded).cloned().collect())
    }

    async fn generate(&self, model: &str, _prompt: &str, _system: Option<&str>) -> Result<String, AppError> {
        if !self.available.load(Ordering::Relaxed) {
            return Err(AppError::External("Mock provider unavailable".to_string()));
        }

        let mut models = self.models.lock().unwrap();
        if let Some(m) = models.iter_mut().find(|m| m.name == model) {
            // Simulate Ollama behavior: model gets loaded into RAM on first generate
            m.is_loaded = true;
        } else {
            return Err(AppError::NotFound(format!("Model '{}' not found", model)));
        }

        Ok(self.response_text.lock().unwrap().clone())
    }

    async fn chat(
        &self,
        model: &str,
        _messages: Vec<ChatMessage>,
        _system: Option<&str>,
    ) -> Result<ChatResponse, AppError> {
        if !self.available.load(Ordering::Relaxed) {
            return Err(AppError::External("Mock provider unavailable".to_string()));
        }

        let models = self.models.lock().unwrap();
        if !models.iter().any(|m| m.name == model) {
            return Err(AppError::NotFound(format!("Model '{}' not found", model)));
        }

        Ok(ChatResponse {
            content: self.response_text.lock().unwrap().clone(),
            done: true,
            total_duration_ns: Some(100_000_000), // 100ms
            prompt_eval_count: Some(10),
            eval_count: Some(20),
        })
    }

    async fn pull_model(&self, model: &str) -> Result<(), AppError> {
        let mut models = self.models.lock().unwrap();
        if models.iter().any(|m| m.name == model) {
            return Ok(()); // Already exists
        }
        models.push(ModelInfo {
            name: model.to_string(),
            size_bytes: 300 * 1024 * 1024,
            quantization: Some("Q4_K_M".to_string()),
            parameter_count: Some("0.5B".to_string()),
            families: vec!["mock".to_string()],
            is_loaded: false,
            modified_at: None,
        });
        Ok(())
    }

    async fn delete_model(&self, model: &str) -> Result<(), AppError> {
        let mut models = self.models.lock().unwrap();
        models.retain(|m| m.name != model);
        Ok(())
    }

    async fn show_model(&self, model: &str) -> Result<ModelInfo, AppError> {
        let models = self.models.lock().unwrap();
        models
            .iter()
            .find(|m| m.name == model)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("Model '{}' not found", model)))
    }

    async fn unload_from_memory(&self, model: &str) -> Result<(), AppError> {
        // Simulate Ollama evicting the model from in-memory cache.
        let mut models = self.models.lock().unwrap();
        if let Some(m) = models.iter_mut().find(|m| m.name == model) {
            m.is_loaded = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_list_running() {
        let provider = MockLocalProvider::new();
        let running = provider.list_running().await.unwrap();
        assert_eq!(running.len(), 0); // Default model is downloaded but not loaded

        // Generate triggers loading (simulates Ollama load-on-first-use)
        provider.generate("mock-model:latest", "hi", None).await.unwrap();
        let running = provider.list_running().await.unwrap();
        assert_eq!(running.len(), 1); // Now loaded

        provider.pull_model("unloaded:latest").await.unwrap();
        let running = provider.list_running().await.unwrap();
        assert_eq!(running.len(), 1); // New model is not loaded
    }
}
