# Research: Existing LLM/AI Infrastructure

**Date**: 2026-02-27
**Purpose**: Analyze existing code to identify integration points for SLM Edge Inference

## Existing LLM Provider Architecture

### Two Parallel LLM Systems

The codebase has **two separate LLM subsystems**:

1. **`conversational_ui/`** — Interactive chat with tool execution
   - Provider trait: `LlmProviderTrait` (`providers/mod.rs:14`)
   - Implementations: `ClaudeProvider`, `OllamaProvider`, `OpenRouterProvider`
   - Used for: Node management chat interface with Lightning tools

2. **`api_store/`** — API marketplace LLM execution
   - No provider trait (uses direct HTTP via `SecureHttpClient`)
   - Execution: `llm_execution.rs` → HTTP call with code templates
   - Fallback: `fallback_service.rs` (037) → Local → Network peer routing
   - Used for: L402-monetized AI API access

### LlmProviderTrait (conversational_ui)

```rust
// File: packages/backend/src/conversational_ui/providers/mod.rs:14
#[async_trait]
pub trait LlmProviderTrait: Send + Sync {
    async fn chat_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        system_prompt: &str,
    ) -> Result<(String, Vec<ToolCall>), AppError>;

    async fn generate_response(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String, AppError> { /* default impl */ }
}
```

### Existing OllamaProvider

**File**: `packages/backend/src/conversational_ui/providers/ollama.rs` (247 lines)

**Capabilities**:
- Connects to `http://localhost:11434/api/chat`
- Supports tool calling (native + text extraction fallback)
- Configurable temperature and max_tokens
- Non-streaming only (stream: false)

**Gaps for SLM work**:
- No memory management (no RAM checks before inference)
- No model lifecycle (no load/unload/swap)
- No model listing (doesn't query /api/tags)
- No health check (doesn't verify Ollama is running)
- Hardcoded tool extraction (only 4 Lightning tools)
- No quantization awareness
- No timeout handling

### LLM Execution Flow (api_store)

```
User Request
  → api_store/service.rs::execute_llm_request()
    → api_store/fallback_service.rs::execute_for_user()
      → Check ExecutionPreference
      → Local: api_store/llm_execution.rs::execute_llm_request() → HTTP to provider
      → On failure: classify_error() → fallback-eligible?
      → Network: find_peers_by_capability() → execute_llm_remote() via L402
      → Return LlmExecuteResponse { execution_source: Local | Network }
```

### Key Types (api_store/models.rs)

```rust
pub enum ExecutionPreference {
    PreferLocal,   // Default: try local first, then network
    LocalOnly,     // Never use network fallback
    NetworkOnly,   // Always use network peers
}

pub enum ExecutionSource {
    Local,
    Network { peer_node_id: String },
}

pub enum LlmErrorType {
    MissingConfiguration, AuthenticationFailure, RateLimitExceeded,
    ProviderTimeout, ProviderUnavailable,  // Fallback-eligible
    InvalidRequest, ContentPolicyViolation, ContextTooLong,
    InsufficientCredits, InternalError,    // Non-fallback
}
```

## Integration Strategy

### Where SLM Router Fits

```
CURRENT FLOW:
  Request → FallbackService → Local (cloud API) → Network Peer

WITH SLM ROUTER:
  Request → ModelRouter → SLM (Ollama local) → Cloud API → Network Peer
                ↓
          MemoryBudgetManager → check RAM
          LifecycleManager → check model loaded
          SlmConfig → check preference
```

The SLM `ModelRouter` sits **above** the existing `FallbackService`, adding a local inference layer before cloud/peer fallback.

### New Module Boundary

```
slm/            ← NEW: Local inference + routing + edge optimization
api_store/      ← EXISTING: Cloud API execution + fallback to peers
conversational_ui/ ← EXISTING: Chat UI with tool execution
```

The `slm/` module is a **peer** to `api_store/` and `conversational_ui/`, not nested inside either. It can route to both:
- `slm/providers/ollama.rs` for local inference
- `api_store/fallback_service.rs` for cloud/peer fallback

### Existing Patterns to Reuse

| Pattern | Source | Reuse |
|---------|--------|-------|
| Feature gate | `tee/config.rs` (TEE_ENABLED) | SLM_ENABLED |
| Config from env | `tee/config.rs::from_env()` | SlmConfig::from_env() |
| Initialize factory | `tee/mod.rs::initialize_providers()` | slm/mod.rs::initialize_slm() |
| Prometheus metrics | `tee/metrics.rs` (lazy_static + prometheus) | slm/metrics.rs |
| HTTP status endpoint | `transport/node_server/http_tee.rs` | http_slm.rs |
| Mock provider | `tee/providers/mock_vls.rs` | slm/providers/mock.rs |
| AppState injection | `state.rs` (Option<Arc<dyn ...>>) | slm_router, slm_lifecycle |

## Radxa Hardware Constraints

- **CPU**: RK3566 ARM64, 4 cores, 1.8GHz. No GPU acceleration for inference.
- **RAM**: 1GB (Radxa Zero 3W). After Linux + backend + LDK: ~700MB free.
- **Storage**: MicroSD 16GB+. Model files: 1-4GB per quantized 7B model.
- **NEON SIMD**: Available for vectorized operations (Ollama uses this automatically).

### Memory Budget Estimation

| Model | FP16 | Q8_0 | Q4_K_M | Fits 512MB? |
|-------|------|------|--------|-------------|
| Llama 3.2 1B | ~2GB | ~1GB | ~600MB | Tight |
| Llama 3.2 3B | ~6GB | ~3GB | ~1.8GB | No |
| Phi-3 Mini (3.8B) | ~7.6GB | ~3.8GB | ~2.2GB | No |
| Qwen2 0.5B | ~1GB | ~500MB | ~300MB | Yes |
| TinyLlama 1.1B | ~2.2GB | ~1.1GB | ~650MB | Tight |

**Conclusion**: Only sub-1B parameter models (Q4) reliably fit in 512MB budget. Recommend `qwen2:0.5b-q4_K_M` or `llama3.2:1b-q4_K_M` as defaults.

## Ollama REST API Reference

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/chat` | POST | Chat inference (our primary endpoint) |
| `/api/generate` | POST | Text generation (alternative) |
| `/api/tags` | GET | List locally available models |
| `/api/show` | POST | Get model info (size, params, quantization) |
| `/api/pull` | POST | Download model from registry |
| `/api/delete` | DELETE | Remove model from local storage |
| `/api/ps` | GET | List currently loaded/running models |
| `/` | GET | Health check (returns "Ollama is running") |

Key: `/api/ps` tells us which models are **loaded into RAM** (vs just downloaded).
