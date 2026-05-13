# Feature Specification: SLM Edge Inference

**Feature Branch**: `039-confidential-compute`
**Created**: 2026-02-27
**Last Updated**: 2026-03-22
**Status**: Implementation (Phase 1 complete, runtime research in progress)
**Foundation**: 039-confidential-compute (TEE security layer)
**Related Specs**: 333-llm-model-discovery (complete), 037-llm-network-fallback (complete), 042-agentic-commerce (MCP integration)

**Input**: Run Small Language Models on edge devices with explicit inference target selection. Client/agent decides where inference runs — no automatic fallback routing.

**Stakeholder Directive (2026-03-12)**: Inference location is decided by the sender/agent. No 3-tier/4-tier automatic fallback. If execution fails, failure handling is the sender's responsibility. Simplify Prometheus metrics for resource-constrained devices.

**Stakeholder Update (2026-03-22)**: NOT just Ollama — Ollama's CUDA dependency makes it unsuitable for most edge devices. Research and support alternative runtimes: llama.cpp (direct, no CUDA), WebLLM (browser-based, zero server), Transformers.js (Node.js runtime). QEMU validation (2026-03-22) confirmed: llama.cpp direct runs Qwen2 0.5B at 53 tok/s on 4GB ARM64, SmolLM2 135M at 153 tok/s on 1GB ARM64 — Ollama runtime overhead makes 1GB deployment infeasible.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Local SLM Inference on Edge Device (Priority: P1)

A node operator runs a small language model locally on their edge device for privacy-preserving inference without depending on cloud APIs. The system manages model loading within strict memory constraints.

**Why this priority**: Local inference is the core differentiator for Decentralized AI. Without it, every query goes to centralized cloud providers.

**Independent Test**: Set `SLM_ENABLED=true SLM_OLLAMA_URL=http://localhost:11434`, start Ollama with a quantized model, send a chat request, verify response comes from local model with `execution_source: Local`.

**Acceptance Scenarios**:

1. **Given** SLM enabled with Ollama running and model loaded, **When** user sends a chat request with `inference_target: Local`, **Then** the system routes to local Ollama and returns response with `execution_source: Local`.
2. **Given** SLM enabled but Ollama not running, **When** user sends a request with `inference_target: Local`, **Then** the system returns an error indicating local inference is unavailable. No automatic fallback to other targets.
3. **Given** SLM enabled with 1GB RAM device, **When** model requires more memory than `SLM_MEMORY_LIMIT_MB`, **Then** the system refuses to load and returns error with available vs required memory.
4. **Given** SLM enabled, **When** node starts, **Then** system checks Ollama availability and loaded models, logging status with target `node_backend::slm`.

---

### User Story 2 — Explicit Inference Target Selection (Priority: P1)

A node operator or AI agent explicitly selects where inference runs: locally on their device (SLM), via a cloud API provider, or on a peer node's local runtime. There is no automatic fallback — the caller chooses the target and handles failure.

**Stakeholder alignment**: "We do not need 3-tier routing. Inference location should be decided by the sender/agent. If execution fails, there is no automatic fallback." — Project stakeholder, 2026-03-12

**Architecture**: Two components collaborate:
1. **SLM module (`ModelRouter`)** — LOCAL-only decision engine. Answers "can this node run this model locally?" based on memory budget, model availability, and context window.
2. **`fallback_service.rs`** — Orchestrator that dispatches to the caller's chosen target. Does NOT cascade between targets.

**Independent Test**: Send requests with different `inference_target` values (Local, Cloud, Peer). Verify each routes to the correct target. Send to unavailable target, verify error returned (no fallback).

**Acceptance Scenarios**:

1. **Given** `inference_target: Local` and local SLM available, **When** user sends a chat request, **Then** system executes on local Ollama.
2. **Given** `inference_target: Cloud` with API key configured, **When** user sends a request, **Then** system routes to the configured cloud provider.
3. **Given** `inference_target: Peer { node_id }` with funded Lightning channel, **When** user sends a request, **Then** system routes to peer's local inference runtime via L402 payment.
4. **Given** `inference_target: Local` but Ollama is down, **When** user sends a request, **Then** system returns error. The caller (user/agent) decides what to do next.
5. **Given** `inference_target: Peer` but peer is offline, **When** user sends a request, **Then** system returns error with peer status information.

---

### User Story 3 — Edge Memory Management (Priority: P2)

A node operator running on a 1GB device needs the system to manage model loading/unloading to prevent out-of-memory crashes. Only one model should be loaded at a time on edge devices. The system tracks real memory usage and enforces hard limits.

**Why this priority**: Memory management prevents device crashes. Without it, loading a 700MB model on a 1GB device while the backend uses 300MB triggers OOM killer, risking Lightning channels.

**Independent Test**: Set `SLM_MEMORY_LIMIT_MB=384`, attempt to load a model requiring 500MB, verify system refuses with clear error.

**Acceptance Scenarios**:

1. **Given** memory limit set to 384MB, **When** model requires 500MB, **Then** system refuses to load with error "Model requires ~500MB but memory budget is 384MB".
2. **Given** one model loaded using 400MB, **When** different model requested, **Then** system unloads current model first, then loads new one (swap).
3. **Given** model loaded, **When** `/api/v2/slm/status` is called, **Then** response includes `memory_used_mb`, `memory_limit_mb`, `loaded_models`.
4. **Given** system memory drops below safety threshold, **When** health check runs, **Then** system logs warning and optionally unloads model.

---

### User Story 4 — Model Lifecycle Management (Priority: P2)

A node operator manages which models are available on their device — pulling new models, listing loaded models, and removing unused models to free storage/memory.

**Why this priority**: Operators need control over their device's model inventory.

**Independent Test**: Use `/api/v2/slm/models` endpoints to list, pull, and remove models. Verify Ollama state matches API responses.

**Acceptance Scenarios**:

1. **Given** Ollama running, **When** operator calls `GET /api/v2/slm/models`, **Then** system returns list of available and loaded models with size and quantization info.
2. **Given** model not yet pulled, **When** operator calls `POST /api/v2/slm/models/pull` with model name, **Then** system pulls model via Ollama API (if memory allows).
3. **Given** model loaded but unused, **When** operator calls `DELETE /api/v2/slm/models/{name}`, **Then** system unloads and removes model.
4. **Given** auto-load configured (`SLM_DEFAULT_MODEL`), **When** backend starts, **Then** default model is loaded into Ollama if not already present.

---

### User Story 5 — TEE-Protected Model Verification (Priority: P3)

A node operator with TEE-enabled hardware verifies that their local SLM model has not been tampered with. The system uses the TPM (from 039-confidential-compute) to store and verify model file hashes.

**Why this priority**: Model integrity verification prevents supply chain attacks on AI models. Requires hardware TEE — runs as mock verification on non-TEE devices.

**Acceptance Scenarios**:

1. **Given** TEE enabled + SLM model loaded, **When** model file hash is computed, **Then** hash is stored in TEE attestation registry.
2. **Given** stored model hash exists, **When** model is loaded again, **Then** system verifies file hash matches stored hash before inference.
3. **Given** model file has been modified, **When** verification runs, **Then** system detects mismatch, logs critical warning, refuses to use model.
4. **Given** TEE disabled, **When** model loads, **Then** hash verification is skipped (graceful degradation).

---

### Edge Cases

- What happens when Ollama crashes mid-inference? System detects connection reset, returns error to caller. Caller decides retry strategy.
- What happens when device runs out of disk space for model storage? Ollama pull fails, system returns error.
- What happens when two requests arrive simultaneously for different models? Queue model swap — only one model at a time on edge. Second request waits or caller gets error.
- What happens when network is down and only local SLM is available? If caller chose Local, it works. If caller chose Cloud/Peer, they get error.
- What happens during model pull and inference request arrives? If model already loaded, use that. If no model loaded, return error.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST support `SLM_ENABLED=false` (default) with zero impact on current functionality.
- **FR-002**: System MUST integrate with Ollama REST API (`/api/chat`, `/api/tags`, `/api/pull`, `/api/delete`) for local inference.
- **FR-003**: System MUST support explicit inference target selection: Local (SLM), Cloud (api_store), or Peer (peer node's local runtime via L402). No automatic fallback between targets.
- **FR-004**: System MUST respect `inference_target` from the caller. If target is unavailable, return error — do not silently switch to another target.
- **FR-005**: System MUST enforce memory budget via `SLM_MEMORY_LIMIT_MB` and refuse model loading when budget exceeded.
- **FR-006**: System MUST support model lifecycle operations: list, load, unload, swap, pull, remove.
- **FR-007**: System MUST support only one loaded model at a time on edge devices (configurable for multi-model on larger devices).
- **FR-008**: System MUST provide `GET /api/v2/slm/status` endpoint with loaded models, memory usage, available inference targets.
- **FR-009**: System MUST provide `GET /api/v2/slm/models` endpoint listing available and loaded models.
- **FR-010**: When inference fails, system MUST return a clear error with failure reason. Caller handles retry/alternative logic.
- **FR-011**: System MUST log all SLM operations with target prefix `node_backend::slm`.
- **FR-012**: System SHOULD expose lightweight metrics for inference operations. Minimize overhead on resource-constrained devices (512MB-1GB).
- **FR-013**: System MUST detect Ollama availability on startup and log status.
- **FR-014**: System MUST auto-load default model (`SLM_DEFAULT_MODEL`) on startup if configured.
- **FR-015**: When TEE enabled (039), system SHOULD verify model file hash against stored hash before inference.
- **FR-016**: System MUST never log model inference inputs or outputs in plaintext (privacy).
- **FR-017**: System MUST support quantized models (Q4_K_M, Q8_0, etc.) with appropriate memory estimation.
- **FR-018**: System MUST provide mock SLM provider for development/testing without Ollama.

### Key Entities

- **InferenceTarget**: Enum — `Local { provider, model }`, `Cloud { platform, model }`, `Peer { node_id, model }`. Caller explicitly selects one. No fallback chain.
- **SlmConfig**: Configuration from env vars — enabled, ollama_url, memory_limit, default_model, max_context.
- **ModelInfo**: Loaded model metadata — name, size_bytes, quantization, parameter_count, loaded_at, families.
- **MemoryBudget**: Current state — used_mb, limit_mb, available_mb, loaded_models.

### Local Inference Runtime Abstraction

The SLM module abstracts local inference runtimes via `LocalInferenceProvider` trait:

```
LocalInferenceProvider trait:
  provider_name() -> &'static str     // e.g. "ollama", "llama-cpp", "mlx"
  is_available()  -> bool             // port reachable, runtime running?
  list_models()   -> Vec<ModelInfo>   // enumerate pulled/loaded models
  load_model()    -> Result           // load model into RAM
  unload_model()  -> Result           // free model from RAM
  infer()         -> Result           // run inference
```

**Current runtime**: Ollama (implemented). Per stakeholder directive (2026-03-22), additional runtimes are now IN SCOPE: llama.cpp (direct binary, no CUDA — validated on QEMU ARM64), WebLLM (browser-based via WebGPU), Transformers.js (Node.js). Ollama's CUDA dependency makes it unsuitable for edge devices without GPU.

### Non-Functional Requirements

- **NFR-001**: Local inference MUST complete within 60s (warm) or 90s (cold) for 100-token prompt on edge device.
- **NFR-002**: Target validation MUST complete within 10ms (pure logic).
- **NFR-003**: Memory tracking MUST update within 1 second of model load/unload.
- **NFR-004**: System MUST not use more than `SLM_MEMORY_LIMIT_MB` for model loading (default 384MB).
- **NFR-005**: Model swap MUST complete within 120s on MicroSD, 90s on eMMC.
- **NFR-006**: `SLM_ENABLED=false` MUST add zero runtime overhead.
- **NFR-007**: SLM module MUST compile on macOS without Ollama installed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Local SLM inference works on development machine with Ollama mock.
- **SC-002**: Explicit target selection correctly dispatches to Local, Cloud, or Peer.
- **SC-003**: Unavailable target returns clear error — no silent fallback.
- **SC-004**: Memory budget enforcement prevents OOM on 1GB simulated environment.
- **SC-005**: Model lifecycle operations (list/load/unload/swap) work against Ollama API.
- **SC-006**: `SLM_ENABLED=false` operates identically to current code with zero overhead.
- **SC-007**: Integration test passes: request with explicit target → correct dispatch → response.

## Technical Constraints

- **TC-001**: Radxa Zero 3W has 1GB RAM minimum. After Linux + backend (~300MB), only ~700MB available.
- **TC-002**: Ollama uses `/api/chat` for inference, `/api/tags` for listing.
- **TC-003**: Quantized models (Q4_K_M) reduce memory ~4x.
- **TC-004**: RK3566 ARM64 CPU has no GPU acceleration. CPU-only with NEON SIMD.
- **TC-005**: Ollama must be installed separately — not embedded in Rust backend.
- **TC-006**: Existing `LlmProviderTrait` in `conversational_ui/providers/mod.rs` must be respected.
- **TC-007**: TEE integration depends on 039-confidential-compute foundation. Security directives apply: fail-closed (hardware failure = reject, not silent downgrade), enforced secure boot (PCR-based policy gating).

## Dependencies

### Software Dependencies

- Rust crates: `reqwest` (HTTP client for Ollama), `sysinfo` (memory monitoring)
- External: Ollama runtime (must be installed separately on target device)
- Internal: 039-confidential-compute (TEE foundation), 042-agentic-commerce (MCP tool exposure)

### Hardware Dependencies

- Radxa Zero 3W (1GB+ RAM, ARM64) — target deployment platform
- Development: Any machine with Ollama installed (or mock provider)

## Out of Scope

- ~~**OOS-001**: Additional `LocalInferenceProvider` runtimes beyond Ollama~~ — **MOVED IN SCOPE** per stakeholder directive 2026-03-22 (llama.cpp, WebLLM, Transformers.js)
- **OOS-002**: GPU acceleration on RK3566
- **OOS-003**: Model fine-tuning on edge devices
- **OOS-004**: Multi-model concurrent inference on edge devices
- **OOS-005**: Model marketplace / distribution network
- **OOS-006**: Output quality evaluation / hallucination detection
- **OOS-007**: Automatic fallback routing between inference targets

---

**See Also**:
- [plan.md](plan.md) — Implementation milestones
- [checklists/tasks.md](checklists/tasks.md) — Task tracking
- [ADR-012: SLM Routing Ownership](../../docs/adr/012-slm-routing-ownership.md) — SLM module responsibilities
- [039-confidential-compute](../039-confidential-compute/) — TEE security foundation
- [042-agentic-commerce](../042-agentic-commerce/) — MCP integration for tool exposure
