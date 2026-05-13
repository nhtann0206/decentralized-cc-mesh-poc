//! Ollama local inference provider with lifecycle awareness.
//!
//! Connects to Ollama REST API at the configured URL for:
//! - Model listing, loading, and management
//! - Chat and generate inference
//! - Memory-aware model lifecycle
//!
//! Separate from `conversational_ui::providers::OllamaProvider` which implements
//! `LlmProviderTrait` for the chat UI. This provider focuses on lifecycle management
//! and raw inference for the SLM routing engine.

use crate::error::AppError;
use crate::slm::models::{ollama_api, ModelInfo};
use crate::slm::providers::{ChatMessage, ChatResponse, LocalInferenceProvider};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, warn};

/// Ollama-backed local inference provider.
pub struct OllamaLocalProvider {
    client: Client,
    base_url: String,
}

impl OllamaLocalProvider {
    /// Create a new provider with the given base URL and HTTP client timeout.
    ///
    /// `timeout_secs` maps to `SLM_OLLAMA_TIMEOUT_SECS` (default: 120).
    /// Applies to all API calls: generate, chat, pull, unload.
    pub fn new(base_url: &str, timeout_secs: u64) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_secs))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Convert Ollama API model to our ModelInfo.
    fn to_model_info(model: &ollama_api::OllamaModel, is_loaded: bool) -> ModelInfo {
        let (quantization, parameter_count, families) = match &model.details {
            Some(d) => (
                d.quantization_level.clone(),
                d.parameter_size.clone(),
                d.families.clone().unwrap_or_default(),
            ),
            None => (None, None, vec![]),
        };

        ModelInfo {
            name: model.name.clone(),
            size_bytes: model.size,
            quantization,
            parameter_count,
            families,
            is_loaded,
            modified_at: Some(model.modified_at.clone()),
        }
    }
}

#[async_trait]
impl LocalInferenceProvider for OllamaLocalProvider {
    fn provider_name(&self) -> &'static str {
        "ollama"
    }

    async fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            AppError::External(format!("Ollama tags request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            return Err(AppError::External(format!(
                "Ollama tags returned {}",
                resp.status()
            )));
        }

        // Get running models to mark loaded status
        let running = self.list_running_names().await.unwrap_or_default();

        let tags: ollama_api::TagsResponse = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama tags: {}", e))
        })?;

        Ok(tags
            .models
            .iter()
            .map(|m| {
                let is_loaded = running.iter().any(|r| r == &m.name);
                Self::to_model_info(m, is_loaded)
            })
            .collect())
    }

    async fn list_running(&self) -> Result<Vec<ModelInfo>, AppError> {
        let url = format!("{}/api/ps", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            AppError::External(format!("Ollama ps request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            return Err(AppError::External(format!(
                "Ollama ps returned {}",
                resp.status()
            )));
        }

        let ps: ollama_api::PsResponse = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama ps: {}", e))
        })?;

        Ok(ps
            .models
            .iter()
            .map(|m| ModelInfo {
                name: m.name.clone(),
                size_bytes: m.size,
                quantization: None,
                parameter_count: None,
                families: vec![],
                is_loaded: true,
                modified_at: None,
            })
            .collect())
    }

    async fn generate(&self, model: &str, prompt: &str, system: Option<&str>) -> Result<String, AppError> {
        let url = format!("{}/api/generate", self.base_url);

        let mut body = json!({
            "model": model,
            "prompt": prompt,
            // Non-streaming: single buffered response.
            // Privacy: prevents token-by-token exposure to logging intermediaries.
            // Simplicity: no SSE/chunked parsing needed; reqwest handles as single JSON body.
            "stream": false,
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }

        debug!(
            target: "node_backend::slm",
            model = model,
            prompt_len = prompt.len(),
            "Sending generate request to Ollama"
        );

        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| {
            AppError::External(format!("Ollama generate request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(
                target: "node_backend::slm",
                status = %status,
                "Ollama generate failed"
            );
            return Err(AppError::External(format!(
                "Ollama generate error {}: {}",
                status, text
            )));
        }

        let result: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama generate response: {}", e))
        })?;

        result["response"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::External("Missing 'response' in Ollama output".to_string()))
    }

    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        system: Option<&str>,
    ) -> Result<ChatResponse, AppError> {
        let url = format!("{}/api/chat", self.base_url);

        let mut ollama_messages: Vec<serde_json::Value> = Vec::new();

        if let Some(sys) = system {
            ollama_messages.push(json!({
                "role": "system",
                "content": sys,
            }));
        }

        for msg in &messages {
            ollama_messages.push(json!({
                "role": msg.role,
                "content": msg.content,
            }));
        }

        let body = json!({
            "model": model,
            "messages": ollama_messages,
            // Non-streaming: same rationale as generate() above.
            "stream": false,
        });

        debug!(
            target: "node_backend::slm",
            model = model,
            message_count = messages.len(),
            "Sending chat request to Ollama"
        );

        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| {
            AppError::External(format!("Ollama chat request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(
                target: "node_backend::slm",
                status = %status,
                "Ollama chat failed"
            );
            return Err(AppError::External(format!(
                "Ollama chat error {}: {}",
                status, text
            )));
        }

        let result: ollama_api::ChatResponse = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama chat response: {}", e))
        })?;

        Ok(ChatResponse {
            content: result.message.content,
            done: result.done,
            total_duration_ns: result.total_duration,
            prompt_eval_count: result.prompt_eval_count,
            eval_count: result.eval_count,
        })
    }

    async fn pull_model(&self, model: &str) -> Result<(), AppError> {
        let url = format!("{}/api/pull", self.base_url);
        let body = json!({
            "name": model,
            "stream": false,
        });

        debug!(
            target: "node_backend::slm",
            model = model,
            "Pulling model from Ollama registry"
        );

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::External(format!("Ollama pull request failed: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Ollama pull failed: {}", text)));
        }

        Ok(())
    }

    async fn delete_model(&self, model: &str) -> Result<(), AppError> {
        let url = format!("{}/api/delete", self.base_url);
        let body = json!({ "name": model });

        let resp = self
            .client
            .delete(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::External(format!("Ollama delete request failed: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Ollama delete failed: {}", text)));
        }

        Ok(())
    }

    async fn unload_from_memory(&self, model: &str) -> Result<(), AppError> {
        let url = format!("{}/api/generate", self.base_url);
        let body = json!({
            "model": model,
            // keep_alive=0 forces Ollama to immediately evict the model from RAM.
            "keep_alive": 0,
        });

        debug!(
            target: "node_backend::slm",
            model = model,
            "Sending unload request to Ollama (keep_alive=0)"
        );

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::External(format!("Ollama unload request failed: {}", e)))?;

        if !resp.status().is_success() {
            warn!(
                target: "node_backend::slm",
                model = model,
                status = %resp.status(),
                "Ollama unload returned non-success (non-fatal)"
            );
        }

        Ok(())
    }

    async fn show_model(&self, model: &str) -> Result<ModelInfo, AppError> {
        let url = format!("{}/api/show", self.base_url);
        let body = json!({ "name": model });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::External(format!("Ollama show request failed: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::External(format!("Ollama show failed: {}", text)));
        }

        let show: ollama_api::ShowResponse = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama show response: {}", e))
        })?;

        let (quantization, parameter_count, families) = match &show.details {
            Some(d) => (
                d.quantization_level.clone(),
                d.parameter_size.clone(),
                d.families.clone().unwrap_or_default(),
            ),
            None => (None, None, vec![]),
        };

        Ok(ModelInfo {
            name: model.to_string(),
            size_bytes: 0, // show doesn't return size
            quantization,
            parameter_count,
            families,
            is_loaded: false, // Can't determine from show alone
            modified_at: None,
        })
    }
}

impl OllamaLocalProvider {
    /// Helper: get names of running models (for marking is_loaded in list_models).
    async fn list_running_names(&self) -> Result<Vec<String>, AppError> {
        let url = format!("{}/api/ps", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            AppError::External(format!("Ollama ps request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let ps: ollama_api::PsResponse = resp.json().await.map_err(|e| {
            AppError::External(format!("Failed to parse Ollama ps: {}", e))
        })?;

        Ok(ps.models.iter().map(|m| m.name.clone()).collect())
    }
}
