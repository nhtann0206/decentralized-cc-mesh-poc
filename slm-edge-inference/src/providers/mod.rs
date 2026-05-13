//! SLM Local Inference Provider trait and implementations.
//!
//! Defines the interface for local model inference (Ollama, llama.cpp, etc.)
//! and provides both a production Ollama provider and a mock for testing.

use crate::error::AppError;
use crate::slm::models::ModelInfo;
use async_trait::async_trait;

pub mod mock;
pub mod ollama;

pub use mock::MockLocalProvider;
pub use ollama::OllamaLocalProvider;

/// Trait for local inference providers (Ollama, llama.cpp, MLX, etc.).
///
/// Separate from `LlmProviderTrait` because local providers need lifecycle
/// awareness (model loading, memory management) that cloud APIs don't have.
///
/// Implementations represent specific edge-inference runtimes. The runtime is an
/// implementation detail — routing decisions use model names, not runtime names.
#[async_trait]
pub trait LocalInferenceProvider: Send + Sync {
    /// Short identifier for this inference runtime (e.g. "ollama", "llama-cpp", "mlx").
    ///
    /// Used in `InferenceTarget::Local { provider }` and observability.
    /// Implementations MUST return a stable, lowercase, hyphen-separated string.
    fn provider_name(&self) -> &'static str;

    /// Check if the provider backend is reachable.
    async fn is_available(&self) -> bool;

    /// List models downloaded on the local system.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError>;

    /// List models currently loaded into RAM.
    async fn list_running(&self) -> Result<Vec<ModelInfo>, AppError>;

    /// Run inference with a prompt. Returns the generated text.
    async fn generate(&self, model: &str, prompt: &str, system: Option<&str>) -> Result<String, AppError>;

    /// Run chat-style inference with message history.
    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        system: Option<&str>,
    ) -> Result<ChatResponse, AppError>;

    /// Pull (download) a model from the registry.
    async fn pull_model(&self, model: &str) -> Result<(), AppError>;

    /// Delete a model from local storage.
    async fn delete_model(&self, model: &str) -> Result<(), AppError>;

    /// Get detailed info about a specific model.
    async fn show_model(&self, model: &str) -> Result<ModelInfo, AppError>;

    /// Evict a model from the provider's in-memory cache without deleting it from disk.
    ///
    /// On Ollama: sends `POST /api/generate` with `keep_alive=0` to force immediate RAM release.
    /// Failure is non-fatal — memory budget tracking still releases even if this fails.
    async fn unload_from_memory(&self, model: &str) -> Result<(), AppError>;
}

/// Chat message for local inference.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Response from local chat inference.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// Generated text content.
    pub content: String,
    /// Whether generation is complete.
    pub done: bool,
    /// Total duration in nanoseconds (if reported by provider).
    pub total_duration_ns: Option<u64>,
    /// Number of tokens evaluated in prompt.
    pub prompt_eval_count: Option<u32>,
    /// Number of tokens generated.
    pub eval_count: Option<u32>,
}
