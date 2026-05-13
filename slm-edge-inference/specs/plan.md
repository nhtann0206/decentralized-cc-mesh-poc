# Implementation Plan: SLM Edge Inference

**Branch**: `039-confidential-compute` | **Date**: 2026-02-27 | **Last Updated**: 2026-03-19
**Spec**: [spec.md](./spec.md)
**Foundation**: 039-confidential-compute (TEE), 042-agentic-commerce (MCP)
**Architecture Reference**: Constitution Principle IX (five-layer architecture)

## Summary

SLM Edge Inference adds local model execution to Lightning Node OS via runtime abstraction. The caller (user or AI agent) explicitly selects the inference target: Local (SLM), Cloud (API), or Peer (L402). There is no automatic fallback — if the chosen target fails, the caller handles it. The system is feature-gated (`SLM_ENABLED=false`) and follows the five-layer architecture.

## Technical Context

**Language/Version**: Rust 1.75+ (backend only)
**Primary Dependencies**: reqwest (runtime HTTP), sysinfo (memory)
**Supported Runtimes** (per stakeholder directive 2026-03-22):
- **Ollama** — implemented, but CUDA dependency makes it unsuitable for most edge devices
- **llama.cpp** (direct) — no CUDA, validated on QEMU ARM64: Qwen2 0.5B @ 53 tok/s (4GB), SmolLM2 135M @ 153 tok/s (1GB)
- **WebLLM** — browser-based via WebGPU, zero server required
- **Transformers.js** — Node.js runtime, good for Bun-based node apps
**Storage**: No new database tables (SLM state is runtime-managed)
**Testing**: cargo test with mock SLM provider (no runtime required)
**Target Platform**: Linux (Radxa Zero 3W ARM64, 1GB RAM), macOS (development)
**Performance Goals**: Target validation < 10ms, inference < 60s (100 tokens), model swap < 120s

## Constitution Check

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Production-Ready Code | PASS | Mock provider fully functional, not a placeholder |
| II. Type Safety | PASS | Enums for InferenceTarget, ModelCapability |
| III. Performance | PASS | Target validation is pure logic (<10ms) |
| IV. API Consistency | PASS | `/api/v2/slm/*` follows ApiResponse wrapper |
| V. Testing | PASS | Mock provider for unit/integration tests |
| VI. UX Consistency | N/A | Backend-only |
| VII. Observability | PASS | `node_backend::slm` target, lightweight metrics |
| VIII. Security | PASS | No inputs/outputs logged. TEE hash verification |
| IX. Layered Architecture | PASS | slm/ module with clear separation |
| X. Frontend Data Fetching | N/A | No frontend in this spec |

## Project Structure

```text
packages/backend/src/slm/
├── mod.rs              # Module exports + initialize_slm()
├── config.rs           # SlmConfig + from_env()
├── models.rs           # InferenceTarget, ModelInfo, MemoryBudget
├── router.rs           # ModelRouter: validates caller's target choice
├── lifecycle.rs        # ModelLifecycleManager: Ollama API integration
├── memory.rs           # MemoryBudgetManager: RAM tracking + enforcement
└── providers/
    ├── mod.rs          # LocalInferenceProvider trait
    ├── ollama.rs       # OllamaLocalProvider
    └── mock.rs         # MockLocalProvider for testing

packages/backend/src/transport/node_server/
└── http_slm.rs         # GET /api/v2/slm/status, GET /models, POST /models/pull

specs/040-slm-edge-inference/
├── README.md           # Navigation
├── spec.md             # Feature specification
├── plan.md             # This plan
└── checklists/
    └── tasks.md        # Task tracking
```

## Implementation Milestones

### Milestone 1: Foundation (Config + Models + Mock Provider) — DONE

| Step | File(s) | Description |
|------|---------|-------------|
| 1.1 | `slm/config.rs` | `SlmConfig` + `from_env()` — SLM_ENABLED, SLM_OLLAMA_URL, SLM_MEMORY_LIMIT_MB |
| 1.2 | `slm/models.rs` | `InferenceTarget`, `ModelInfo`, `MemoryBudget`, `ModelCapability` |
| 1.3 | `slm/providers/mod.rs` | `LocalInferenceProvider` trait |
| 1.4 | `slm/providers/mock.rs` | `MockLocalProvider` with configurable responses |
| 1.5 | `slm/mod.rs` | Module exports + `initialize_slm()` factory |

### Milestone 2: Memory Management + Model Lifecycle — DONE

| Step | File(s) | Description |
|------|---------|-------------|
| 2.1 | `slm/memory.rs` | `MemoryBudgetManager` — read system memory, enforce limits |
| 2.2 | `slm/lifecycle.rs` | `ModelLifecycleManager` — load/unload/swap via Ollama REST |
| 2.3 | `slm/providers/ollama.rs` | `OllamaLocalProvider` with lifecycle awareness |

### Milestone 3: Target Validation + Dispatch — DONE

| Step | File(s) | Description |
|------|---------|-------------|
| 3.1 | `slm/router.rs` | `ModelRouter` — validates caller's explicit target choice (can this target handle it?) |
| 3.2 | Integration | Connect to `LlmFallbackService` for dispatch (no cascade, explicit target only) |

### Milestone 4: HTTP Endpoints — DONE

| Step | File(s) | Description |
|------|---------|-------------|
| 4.1 | `http_slm.rs` | `/api/v2/slm/status`, `/models`, `/models/pull` |
| 4.2 | `state.rs` | Wire SLM into AppState + bootstrap |

### Milestone 5: TEE Integration (Pending Hardware)

| Step | File(s) | Description |
|------|---------|-------------|
| 5.1 | `slm/mod.rs` | Model hash verification using 039 `AttestationRegistry` |

---

## Dependencies

**Milestone 1 → 2**: Config needed for memory limits
**Milestone 2 → 3**: Lifecycle needed for model availability checks
**Milestone 3 → 4**: Router needed for status endpoint data
**Milestone 5**: Depends on 039 hardware (can run with mock TEE)

**External**: Ollama runtime (development only, not required for mock tests)

---

**See Also**:
- [spec.md](spec.md) — Feature specification
- [checklists/tasks.md](checklists/tasks.md) — Task tracking
