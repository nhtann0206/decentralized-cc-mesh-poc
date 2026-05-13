use serde::{Deserialize, Serialize};

/// Where inference will be executed.
#[derive(Debug, Clone, Serialize)]
pub enum InferenceTarget {
    /// Local edge-SLM via this node's inference runtime (free, private, higher latency).
    ///
    /// `provider` is the runtime identifier (e.g. "ollama", "llama-cpp", "mlx").
    /// Runtime is an implementation detail — callers should not branch on it.
    Local {
        provider: String,
        model: String,
    },
    /// Cloud API via api_store (fast, paid).
    Cloud {
        platform: String,
        model: String,
    },
    /// Peer node's edge-SLM (L402 micropayment, decentralized, private).
    ///
    /// The peer's runtime (Ollama, llama.cpp, etc.) is their implementation detail.
    /// Discovery uses Sprint 037 peer network with `platform: "edge-slm"`.
    /// NOTE: Peer SLM routing is future scope — data type exists to represent the vision.
    Peer {
        node_id: String,
        model: String,
    },
}

impl InferenceTarget {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    pub fn is_cloud(&self) -> bool {
        matches!(self, Self::Cloud { .. })
    }

    pub fn is_peer(&self) -> bool {
        matches!(self, Self::Peer { .. })
    }

    pub fn target_type(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Cloud { .. } => "cloud",
            Self::Peer { .. } => "peer",
        }
    }
}

/// Metadata about a model available in Ollama.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g., "qwen2:0.5b-q4_K_M").
    pub name: String,
    /// Total size in bytes.
    pub size_bytes: u64,
    /// Quantization level (e.g., "Q4_K_M", "Q8_0", "F16").
    pub quantization: Option<String>,
    /// Parameter count (e.g., "0.5B", "1B", "3B").
    pub parameter_count: Option<String>,
    /// Model families (e.g., ["qwen2", "transformer"]).
    pub families: Vec<String>,
    /// Whether the model is currently loaded into RAM.
    pub is_loaded: bool,
    /// When the model was last modified/pulled.
    pub modified_at: Option<String>,
}

impl ModelInfo {
    /// Estimate RAM usage in MB based on model size.
    /// R-009 §4.6 measured actual overhead at 19-33% for qwen2:0.5b-q4_K_M
    /// (KV cache ~24MB at 2048 ctx + compute buffers ~40MB + thread buffers ~12MB).
    /// 25% is the recommended safe middle ground.
    pub fn estimated_ram_mb(&self) -> u64 {
        let base_mb = self.size_bytes / (1024 * 1024);
        // 25% overhead: KV cache + compute scratch + thread-local buffers (R-009 §4.7)
        base_mb + (base_mb * 25 / 100)
    }
}

/// Result of a routing decision.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingDecision {
    /// Selected inference target.
    pub target: InferenceTarget,
    /// Human-readable reason for the decision.
    pub reason: String,
    /// Estimated latency in milliseconds.
    pub estimated_latency_ms: Option<u64>,
}

/// Current memory budget state.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryBudget {
    /// Memory currently used by loaded models (MB).
    pub used_mb: u64,
    /// Configured memory limit (MB).
    pub limit_mb: u64,
    /// Available memory for new models (MB).
    pub available_mb: u64,
    /// Names of currently loaded models.
    pub loaded_models: Vec<String>,
}

/// SLM subsystem status for the status endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct SlmStatus {
    /// Whether SLM is enabled.
    pub enabled: bool,
    /// Active inference runtime identifier (e.g. "ollama", "llama-cpp", "mock").
    /// Empty string when SLM is disabled.
    pub provider: String,
    /// Whether the local inference runtime is reachable.
    /// Field name kept as `ollama_available` for HTTP API contract stability;
    /// reflects the active provider regardless of which runtime it is.
    pub ollama_available: bool,
    /// Inference runtime endpoint URL (HTTP-based providers only).
    /// Field name kept as `ollama_url` for HTTP API contract stability.
    pub ollama_url: String,
    /// Memory budget state.
    pub memory: MemoryBudget,
    /// Available models (loaded and downloaded).
    pub models: Vec<ModelInfo>,
    /// Configured default model.
    pub default_model: Option<String>,
}

/// Request to pull a model from Ollama registry.
#[derive(Debug, Clone, Deserialize)]
pub struct PullModelRequest {
    /// Model name to pull (e.g., "qwen2:0.5b-q4_K_M").
    pub model: String,
}

/// Ollama API response types for deserialization.
pub mod ollama_api {
    use serde::Deserialize;

    /// Response from GET /api/tags
    #[derive(Debug, Deserialize)]
    pub struct TagsResponse {
        pub models: Vec<OllamaModel>,
    }

    #[derive(Debug, Deserialize)]
    pub struct OllamaModel {
        pub name: String,
        pub size: u64,
        pub modified_at: String,
        pub details: Option<OllamaModelDetails>,
    }

    #[derive(Debug, Deserialize)]
    pub struct OllamaModelDetails {
        pub format: Option<String>,
        pub family: Option<String>,
        pub families: Option<Vec<String>>,
        pub parameter_size: Option<String>,
        pub quantization_level: Option<String>,
    }

    /// Response from GET /api/ps
    #[derive(Debug, Deserialize)]
    pub struct PsResponse {
        pub models: Vec<RunningModel>,
    }

    #[derive(Debug, Deserialize)]
    pub struct RunningModel {
        pub name: String,
        pub size: u64,
        pub size_vram: Option<u64>,
    }

    /// Response from POST /api/show
    #[derive(Debug, Deserialize)]
    pub struct ShowResponse {
        pub modelfile: Option<String>,
        pub parameters: Option<String>,
        pub template: Option<String>,
        pub details: Option<OllamaModelDetails>,
    }

    /// Response from POST /api/chat
    #[derive(Debug, Deserialize)]
    pub struct ChatResponse {
        pub message: ChatMessage,
        pub done: bool,
        pub total_duration: Option<u64>,
        pub load_duration: Option<u64>,
        pub prompt_eval_count: Option<u32>,
        pub eval_count: Option<u32>,
        pub eval_duration: Option<u64>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ChatMessage {
        pub role: String,
        pub content: String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_estimated_ram() {
        let model = ModelInfo {
            name: "qwen2:0.5b-q4_K_M".to_string(),
            size_bytes: 300 * 1024 * 1024, // 300MB file
            quantization: Some("Q4_K_M".to_string()),
            parameter_count: Some("0.5B".to_string()),
            families: vec!["qwen2".to_string()],
            is_loaded: false,
            modified_at: None,
        };
        // 300MB + 25% overhead = 375MB (R-009 §4.7)
        assert_eq!(model.estimated_ram_mb(), 375);
    }

}
