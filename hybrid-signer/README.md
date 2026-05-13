# HybridSigner — Lightning Channel Keys in Hardware

## What this is

A custom signer for the Lightning Development Kit (LDK) that holds the channel-private seed inside a hardware secure element (or a TEE-backed secure storage), instead of in process memory where any compromise of the operator's host would walk away with funds.

## The problem it solves

LDK ships with a default signer (`KeysManager`) that keeps channel keys in process memory. That is fine for a single-operator service behind a corporate perimeter; it is not fine for a mesh where any home-node operator can route real Lightning payments and operators are not security professionals. If a renter is going to trust a stranger's Raspberry Pi with their Lightning channel keys, the keys need to be in silicon, not in RAM.

## What it does

- **Polymorphic signer.** The signer is an enum — `InMemory` for development, `Hardware` for production. The hardware path delegates to whichever attestation provider the operator wired up (secure element via I2C, TEE-backed secure storage, etc.). LDK upstream is not forked; the adapter implements LDK's signer traits and is injected via `NodeBuilder`.
- **Differential fail mode under hardware degradation.** A circuit breaker watches the hardware signing path. If it trips, big-money HTLCs (above an operator-configurable threshold) are *refused* — fail closed. Tiny routing fees fall back to in-memory signing — fail open. The node stays useful as a router even when the secure element flakes, but it cannot be tricked into signing large outgoing payments without an intact hardware path. The operator sees the degradation in Prometheus metrics; the renter sees an attestation status drop in the UI.
- **Sync↔async bridge without livelock.** LDK's signer API is synchronous and runs on LDK's background-processor thread. Hardware secure elements typically expose async transports (I2C/SPI) that take tens of milliseconds. Naively bridging them deadlocks the runtime. The signer pins all hardware signing onto a dedicated blocking thread pool with a bounded queue back to the runtime. This is documented as its own research output because the obvious approaches all break in non-obvious ways.
- **Post-restart channel verification.** Before the node accepts any new HTLCs after a restart, the signer re-attests against the hardware key and verifies each active channel's commitment number matches what LDK persisted. Mismatch means refuse the channel and prompt the operator. This is the defence against "swap the SD card during a power cycle" attacks against home nodes.
- **Memory hygiene.** The seed never lives in a plain `[u8]`. It's wrapped in `Zeroizing<>` so the bytes are wiped on drop, eliminating the class of bugs where a freed allocation retains key material until the page is reused.

## Why this is hard

There is no published reference design for the combination of (1) staying upstream-compatible with LDK, (2) bridging sync↔async signing without livelocking the runtime, (3) failing differentially for big vs small payments, (4) recovering safely from restart. Every individual piece has prior art; the assembled system does not. The research note in [`docs/spawn_blocking_strategy.md`](docs/spawn_blocking_strategy.md) is the writeup of what survived adversarial review of the obvious approaches.

The security model went through one significant pivot — the original substrate of choice was an ESP32-based secure element; threat-model analysis dropped that in favour of an ATECC608A. The spec docs in [`specs/`](specs/) capture both the model and the pivot.

## Key files

- [`src/ldk_signer_adapter.rs`](src/ldk_signer_adapter.rs) — the `HybridSigner` enum and `TeeSignerFactory` that wires it into LDK.
- [`src/circuit_breaker.rs`](src/circuit_breaker.rs) — closed/open/half-open state machine, threshold logic, fail-mode policy.
- [`src/recovery.rs`](src/recovery.rs) — post-restart channel verification.
- [`src/metrics.rs`](src/metrics.rs) — Prometheus metrics so operators can see the hardware path degrading before it incident-escalates.
- [`docs/spawn_blocking_strategy.md`](docs/spawn_blocking_strategy.md) — the sync↔async research note.
- [`specs/SECURITY_MODEL.md`](specs/SECURITY_MODEL.md) — the threat model, the HTLC threshold rationale, the ESP32→ATECC608A pivot.
- [`specs/ARCHITECTURE.md`](specs/ARCHITECTURE.md), [`specs/ROADMAP.md`](specs/ROADMAP.md), [`specs/plan.md`](specs/plan.md) — surrounding design docs.
