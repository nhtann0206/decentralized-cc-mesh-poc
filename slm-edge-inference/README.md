# SLM Edge Inference Module

**When**: 2026-02-27, 1 day. Then revisited 2026-03-31 ("simplified per directive") when the stakeholder cut the 3-tier fallback design.

## What this is

Small Language Model inference running on the node itself — the alternative to "every prompt phones home to OpenAI." Operators set memory budgets (e.g. "give me 4 GB for SLM"), the module picks which models fit, manages their lifecycle (load on first request, evict under memory pressure, swap on demand), and exposes them on the API surface as just-another-provider so the agent framework calls them identically to a cloud LLM.

## What's here

- [`src/router.rs`](src/router.rs) — model routing: which physical model serves a given (capability, max-tokens, latency-class) request. Originally part of a 3-tier fallback (local → peer → cloud) that the stakeholder later cut — per the spec doc, the inference location is now decided by the *sender/agent*, not auto-fallback. So this is the "given the operator's chosen tier, pick the model" router only.
- [`src/lifecycle.rs`](src/lifecycle.rs) — load/evict/swap state machine. Tracks `Loading | Ready | Evicting | Failed` per model handle, exposes idempotent operations, drains in-flight requests before evicting.
- [`src/memory.rs`](src/memory.rs) — memory accounting. Operators on 8 GB Pi 5s cannot afford accidentally double-loading a model.
- [`src/metrics.rs`](src/metrics.rs) — Prometheus metrics (load time, eviction count, queue depth, tokens/sec).
- [`src/providers/ollama.rs`](src/providers/ollama.rs) — first real provider. Wraps Ollama's HTTP API behind the `SlmProvider` trait. Handles streaming, cancellation, errors. ~400 lines.
- [`src/providers/mock.rs`](src/providers/mock.rs) — deterministic mock for tests.
- [`src/transport/http_slm.rs`](src/transport/http_slm.rs) — `/api/slm/*` HTTP endpoints (list models, generate, stream).
- [`specs/spec.md`](specs/spec.md), [`specs/plan.md`](specs/plan.md), [`specs/research.md`](specs/research.md) — Spec 040. `research.md` is worth reading for the "why Ollama on a 1 GB node is infeasible, llama.cpp direct preferred" trade-off analysis.

## Architectural note

The original design was a 3-tier auto-fallback router (local SLM → peer SLM via gossip → cloud LLM). The stakeholder cut this on 2026-03-12 with the directive: *"We do not need 3-tier routing. Inference location should be decided by the sender/agent. If execution fails, there is no automatic fallback."* (See `specs/spec.md`.) That's a strong-opinion call — auto-fallback creates non-obvious cost paths in a Lightning-priced mesh — and the simplification it forced (single-tier execution, fail-closed) made the whole thing easier to reason about. Worth keeping in mind when seeing the "3-tier fallback" mention in the early Feb commit messages.

## Commits

`bc081d7b` (module core: routing, lifecycle, memory, metrics), `6ab9d27f` (bootstrap + HTTP + AppState), `27d0cf08` (`ModelRouter` integration into `LlmFallbackService`), `70250280` (Spec 040), `b6f06b38` (2026-03-31 — simplified per directive after 3-tier cut).
