# HybridSigner — LDK custom signer with hardware-backed channel keys

**When**: 2026-02-23 → 2026-03-03. ~9 days of design + integration + hardening.

The single most security-sensitive piece of code in this project, and the one I'm proudest of from this arc.

## The problem

LDK (the Rust Lightning Development Kit) ships `KeysManager` — a default signer that holds the channel-private keys *in process memory*. If an operator's box is compromised, the attacker walks away with funds. For a decentralized mesh where any home-node operator can route real Lightning payments, that's unacceptable.

We needed a signer that:
1. Holds the seed in *hardware* (ATECC608A secure element via I2C, or a TEE-backed secure storage) rather than process memory.
2. Drops into LDK's `NodeBuilder` *without* forking LDK irreparably.
3. Survives the impedance mismatch between LDK's **sync** signer API and `ATECC608A`'s **async** I2C transport.
4. Fails *closed* (no signing when hardware is unhealthy) for HTLCs above a threshold, but fails *open* (in-memory fallback) for tiny routing fees — so the node stays useful even if the secure element flakes.

## What's here

- [`src/ldk_signer_adapter.rs`](src/ldk_signer_adapter.rs) — `HybridSigner` enum (`InMemory(KeysManager)` | `Hardware(TeeSigner)`) + `TeeSignerFactory` that wires it into LDK's `NodeBuilder`. The adapter pattern keeps LDK upstream-compatible — we don't fork LDK's signer; we implement `EntropySource + NodeSigner + SignerProvider` ourselves and hand LDK an instance.
- [`src/circuit_breaker.rs`](src/circuit_breaker.rs) — closed/open/half-open state machine guarding the hardware path. Trips on 5 consecutive `ATECC608A` failures within 60s, half-opens after 30s. While open: every HTLC > `htlc_threshold_msat` is rejected (fail-closed for big money); HTLCs ≤ threshold fall back to in-memory signing (fail-open for routing fees). Threshold lives in `TeeConfig`, not hardcoded.
- [`src/metrics.rs`](src/metrics.rs) — Prometheus metrics. Sign latency p50/p95/p99, circuit-breaker state, hardware-vs-memory split, fallback count. Operators need to *see* the trust degradation in real time, not after an incident.
- [`src/recovery.rs`](src/recovery.rs) — post-restart channel verification (T-705). After a node restart, before accepting new HTLCs, the signer re-attests against the hardware key and verifies each active channel's `commitment_number` matches what LDK persisted. Mismatch = refuse channel; prompt operator. This catches the "swap an SD card mid-restart" class of attack.
- [`docs/spawn_blocking_strategy.md`](docs/spawn_blocking_strategy.md) — R-002 research output. LDK's signer API is sync, runs inside LDK's `BackgroundProcessor` thread. ATECC608A I2C is async (~12-40ms per sign on a Pi 5). Naive `block_on(async_sign)` from inside an async runtime worker thread = deadlock. Resolution: pin signing onto a dedicated `spawn_blocking` thread pool, with a bounded mpsc queue back to the main runtime. This doc is the writeup of why the obvious approaches don't work and which one survived adversarial review.
- [`specs/SECURITY_MODEL.md`](specs/SECURITY_MODEL.md), [`specs/ARCHITECTURE.md`](specs/ARCHITECTURE.md), [`specs/ROADMAP.md`](specs/ROADMAP.md), [`specs/plan.md`](specs/plan.md) — Spec 039 documents. SECURITY_MODEL.md is the most useful as a standalone read: threat model, signer trust boundary, HTLC threshold rationale, fallback policy, the ATECC608A vs ESP32 decision (we pivoted to ATECC608A — see commit `e07ba0a4`, 2026-02-25 — after the ESP32 path didn't meet the threat model).

## Why this is hard

The combination of (1) staying upstream-compatible with LDK, (2) bridging sync↔async without livelocking the runtime, (3) failing differentially for big vs small payments, and (4) recovering safely from restart — there is **no published reference design** for this. The R-002 doc is what you get when you have to invent the pattern under deadline pressure and want to be honest about why you chose what you chose. The `Zeroizing<>` wrap on the seed (commit `28dcb6cc`, 2026-03-03) is the kind of detail you only catch when you do an adversarial review of your own code after sleeping on it.

## Commits

`c807916e` (M4 spec), `45047a80` (TeeSignerFactory wired into LDK NodeBuilder), `52f26fd8` (circuit breaker), `af8efa07` (EventBus integration), `f77cd549` (Prometheus metrics), `e9003172` (post-restart channel verification T-705), `f296fd42` (spawn_blocking R-002 doc), `28dcb6cc` (Zeroizing<> seed wrap).
