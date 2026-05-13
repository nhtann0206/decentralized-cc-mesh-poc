/// Default Ollama endpoint.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Memory budget for the SLM subsystem on edge device.
/// **Validated by R-009** — on Radxa Zero 3W (1GB RAM), worst-case available for
/// models is ~502MB after OS + backend + LDK + Ollama daemon. 384MB provides
/// 118MB safety margin to avoid OOM killing LDK (Lightning channel fund loss).
/// Operators with 2GB+ RAM devices should increase via SLM_MEMORY_LIMIT_MB.
/// See: docs/conf-com/research/slm/memory_budget_profiling.md §3.6, §8.2
pub const DEFAULT_MEMORY_LIMIT_MB: u64 = 384;

/// Maximum context window (tokens) per inference request.
/// **Unvalidated assumption** — pending R-007/R-008 model benchmarks on Radxa.
/// Larger contexts increase KV-cache size and may exceed the 384 MB memory budget.
/// Adjust via SLM_MAX_CONTEXT_TOKENS after R-009 memory verdict.
pub const DEFAULT_MAX_CONTEXT_TOKENS: u32 = 2048;

/// Default model for edge inference (Ollama registry format).
/// qwen2:0.5b-q4_K_M — chosen for smallest known footprint; **no quality comparison done yet**.
/// Pending R-008 model quality comparison before changing.
/// Final choice depends on R-009 memory verdict (what fits in budget).
pub const DEFAULT_MODEL: &str = "qwen2:0.5b-q4_K_M";

/// Default swap_model() timeout in seconds.
/// 90s covers: unload (~5s) + load via generate (~60s on Radxa MicroSD) + buffer.
/// Pending R-007 hardware benchmarks for calibration.
pub const DEFAULT_SWAP_TIMEOUT_SECS: u64 = 90;

/// Default Ollama HTTP client timeout in seconds.
/// 120s: generous for model pull/load on slow hardware. Configurable via
/// SLM_OLLAMA_TIMEOUT_SECS for environments with faster storage (eMMC, NVMe).
pub const DEFAULT_OLLAMA_TIMEOUT_SECS: u64 = 120;

// ── 041-peer-ai-service: Provider-side defaults ──

/// Default fee in sats per inference request when serving peers.
/// Based on R-010 findings (~50-100 sats per ~2000 token request for cloud).
/// Edge SLM is cheaper: 5 sats per request as baseline incentive.
pub const DEFAULT_FEE_SATS_PER_REQUEST: u32 = 5;

/// Maximum concurrent peer inference requests this node will accept.
/// Edge devices (1GB RAM) can barely handle 1 model + 1 inference.
/// Default 2 allows minimal queuing; excess gets 503 Service Unavailable.
pub const DEFAULT_MAX_CONCURRENT_PEER_REQUESTS: u32 = 2;

/// SLM configuration loaded from environment variables.
///
/// - `SLM_ENABLED`: "true" or "false" (default: "false")
/// - `SLM_OLLAMA_URL`: Ollama HTTP endpoint (default: "http://localhost:11434")
/// - `SLM_MEMORY_LIMIT_MB`: Max RAM for model loading (default: 384)
/// - `SLM_DEFAULT_MODEL`: Model to auto-load on startup (default: "qwen2:0.5b-q4_K_M")
/// - `SLM_MAX_CONTEXT_TOKENS`: Context window limit (default: 2048)
/// - `SLM_SWAP_TIMEOUT_SECS`: Timeout for swap_model() in seconds (default: 90)
/// - `SLM_OLLAMA_TIMEOUT_SECS`: Ollama HTTP client timeout in seconds (default: 120)
/// - `SLM_PEER_SERVICE_ENABLED`: Advertise SLM to peers via gossip (default: false)
/// - `SLM_FEE_SATS_PER_REQUEST`: Fee charged per peer inference request (default: 5)
/// - `SLM_MAX_CONCURRENT_PEER_REQUESTS`: Max concurrent peer inferences (default: 2)
#[derive(Clone, Debug)]
pub struct SlmConfig {
    pub enabled: bool,
    pub ollama_url: String,
    pub memory_limit_mb: u64,
    pub default_model: Option<String>,
    pub max_context_tokens: u32,
    pub swap_timeout_secs: u64,
    pub ollama_timeout_secs: u64,
    /// When true, this node advertises its loaded SLM models to peers via gossip
    /// and accepts inference requests from other nodes. Requires SLM_ENABLED=true.
    pub peer_service_enabled: bool,
    /// Fee in sats charged per peer inference request (041-peer-ai-service)
    pub fee_sats_per_request: u32,
    /// Maximum concurrent peer inference requests (excess returns 503)
    pub max_concurrent_peer_requests: u32,
}

impl SlmConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("SLM_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            == "true";

        let ollama_url = std::env::var("SLM_OLLAMA_URL")
            .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_string());

        let memory_limit_mb = std::env::var("SLM_MEMORY_LIMIT_MB")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_MEMORY_LIMIT_MB);

        let default_model = std::env::var("SLM_DEFAULT_MODEL").ok();

        let max_context_tokens = std::env::var("SLM_MAX_CONTEXT_TOKENS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS);

        let swap_timeout_secs = std::env::var("SLM_SWAP_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_SWAP_TIMEOUT_SECS);

        let ollama_timeout_secs = std::env::var("SLM_OLLAMA_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_OLLAMA_TIMEOUT_SECS);

        let peer_service_enabled = std::env::var("SLM_PEER_SERVICE_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            == "true";

        let fee_sats_per_request = std::env::var("SLM_FEE_SATS_PER_REQUEST")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_FEE_SATS_PER_REQUEST);

        let max_concurrent_peer_requests = std::env::var("SLM_MAX_CONCURRENT_PEER_REQUESTS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_PEER_REQUESTS);

        Self {
            enabled,
            ollama_url,
            memory_limit_mb,
            default_model,
            max_context_tokens,
            swap_timeout_secs,
            ollama_timeout_secs,
            peer_service_enabled,
            fee_sats_per_request,
            max_concurrent_peer_requests,
        }
    }
}

impl Default for SlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            memory_limit_mb: DEFAULT_MEMORY_LIMIT_MB,
            default_model: None,
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
            swap_timeout_secs: DEFAULT_SWAP_TIMEOUT_SECS,
            ollama_timeout_secs: DEFAULT_OLLAMA_TIMEOUT_SECS,
            peer_service_enabled: false,
            fee_sats_per_request: DEFAULT_FEE_SATS_PER_REQUEST,
            max_concurrent_peer_requests: DEFAULT_MAX_CONCURRENT_PEER_REQUESTS,
        }
    }
}