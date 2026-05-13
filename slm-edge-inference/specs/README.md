# Spec 040 — SLM Edge Inference

**Branch**: `039-confidential-compute`
**Status**: Phase 1 Complete (Milestones 1-4 done, Milestone 5 pending hardware)
**Last Updated**: 2026-03-19

## Overview

Local Small Language Model (SLM) inference on edge devices via Ollama integration. The caller (user or AI agent) explicitly selects the inference target — Local, Cloud, or Peer. No automatic fallback routing.

## Specs

- [spec.md](spec.md) — Feature specification with user stories and requirements
- [plan.md](plan.md) — Implementation milestones with constitution check
- [checklists/tasks.md](checklists/tasks.md) — Task tracking

## Architecture

```
Caller (User / AI Agent)
    │
    ├── inference_target: Local
    │   └── SLM Module (slm/router.rs)
    │       ├── ModelRouter → validates target feasibility
    │       ├── MemoryBudgetManager → enforce 384MB limit
    │       ├── ModelLifecycleManager → load/unload/swap
    │       └── OllamaProvider → localhost:11434
    │
    ├── inference_target: Cloud
    │   └── api_store/service.rs → HTTP to cloud provider
    │
    └── inference_target: Peer { node_id }
        └── L402 payment → peer's local inference runtime
```

**No fallback**: If the chosen target is unavailable, the system returns an error. The caller decides what to do next.

## Configuration

```env
SLM_ENABLED=false              # Feature gate (default: disabled)
SLM_OLLAMA_URL=http://localhost:11434
SLM_MEMORY_LIMIT_MB=384        # Max RAM for model loading
SLM_DEFAULT_MODEL=qwen2:0.5b-q4_K_M
SLM_MAX_CONTEXT_TOKENS=2048    # Context window limit
```

## Related Specs

| Spec | Relationship |
|------|-------------|
| [039-confidential-compute](../039-confidential-compute/) | TEE foundation, model hash verification |
| [042-agentic-commerce](../042-agentic-commerce/) | MCP tool exposure for inference |
| [333-llm-model-discovery](../333-llm-model-discovery/) | Cloud model discovery |
