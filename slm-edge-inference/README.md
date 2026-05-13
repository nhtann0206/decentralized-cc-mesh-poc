# Small Language Model Inference — On-Node, Metered

## What this is

The module that lets a home node sell AI inference to other nodes, running the models on its own hardware instead of relaying calls to a cloud LLM. Inference is treated the same as any other capability in the mesh: priced, paid in sats, observable, fail-closed.

## What it does

- **Per-operator memory budget.** Operators say "give me 4 GB for SLM" and the module decides which models from the catalogue fit, loads them on first request, and evicts under memory pressure.
- **Lifecycle state machine.** Loading, ready, evicting, failed. Idempotent operations. Drains in-flight requests before evicting so callers never see a half-killed model.
- **Provider abstraction.** The first real provider wraps Ollama's HTTP API (streaming, cancellation, error handling, ~400 LOC of network plumbing); a mock provider keeps CI honest. A llama.cpp-direct provider is a future drop-in.
- **HTTP API surface** (`/api/slm/*`) that exposes the catalogue, generation, and streaming endpoints to the rest of the system.
- **Prometheus metrics** so an operator can see model load times, eviction counts, queue depth, and tokens/sec — the things that actually matter for capacity planning on a constrained box.

## The directive that shaped it

The original design was a three-tier auto-fallback router: local SLM first, then a peer SLM over gossip, then a cloud LLM if the first two failed. The project stakeholder cut that mid-design with the line *"inference location should be decided by the sender/agent. If execution fails, there is no automatic fallback."* That was the correct call — auto-fallback in a Lightning-priced mesh creates non-obvious cost paths the user never opts into, and a "the call quietly fell through to OpenAI" outcome is the worst kind of failure mode. The module simplified to single-tier execution with fail-closed semantics, which makes the system easier to reason about and easier for operators to price. The spec doc captures both the original design and the simplification.

## Key files

- [`src/router.rs`](src/router.rs) — "given the operator's chosen tier, pick the model that satisfies (capability, max-tokens, latency-class)."
- [`src/lifecycle.rs`](src/lifecycle.rs) — the load/evict/swap state machine.
- [`src/memory.rs`](src/memory.rs) — memory accounting. The thing that stops you from accidentally double-loading a model on a 4 GB Pi.
- [`src/providers/ollama.rs`](src/providers/ollama.rs) — the first production provider.
- [`src/transport/http_slm.rs`](src/transport/http_slm.rs) — the HTTP surface.
- [`specs/spec.md`](specs/spec.md), [`specs/research.md`](specs/research.md) — the design spec and the "why Ollama on a 1 GB node is infeasible, llama.cpp preferred for tiny operators" research.
