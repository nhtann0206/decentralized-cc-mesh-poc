# TEE Foundation Module

**When**: 2026-02-15 (onboarding week) — built from scratch in 2 days.

The Confidential Compute substrate for the project. Before this module existed there was no concept of hardware-rooted attestation in the backend at all; this is what every later CC piece (Phase A SEV-SNP verifier, Phase C OP-TEE TA, HybridSigner) plugged into.

## What's here

- [`src/traits.rs`](src/traits.rs) — `HardwareTrust` + `VlsProvider` trait definitions. The two pluggable abstractions: one for the chip-rooted identity primitive (attest a measurement against a hardware key), one for validating Lightning state-machine transitions (VLS = Validating Lightning Signer, the trust boundary for HTLC threshold enforcement). Both polymorphic — mock provider for CI/dev, real provider for hardware boards. This trait shape is what kept Phase A (AMD), Phase C (ARM), and any future RISC-V/Apple Silicon target drop-in compatible.
- [`src/error.rs`](src/error.rs) — typed error hierarchy. No `String` errors.
- [`src/config.rs`](src/config.rs) — `TeeConfig` with HTLC threshold, attestation refresh interval, circuit-breaker tuning. Refactored from hardcoded constants (C-009) the day after the first commit.
- [`src/providers/mock_hardware.rs`](src/providers/mock_hardware.rs), [`mock_vls.rs`](src/providers/mock_vls.rs) — deterministic mock implementations so CI can exercise the attestation path without a chip on the bench. Mock = fixed key + fixed measurement; tests assert flow, not crypto strength.
- [`src/http_tee.rs`](src/http_tee.rs) — HTTP transport for `/api/tee/attest`, `/api/tee/status`, `/api/tee/circuit-breaker`. This is what the rental flow calls to fetch a fresh attestation report from the operator. Sits in the Transport layer; delegates to the service via the `HardwareTrust` trait.
- [`src/mod.rs`](src/mod.rs) — module root + lifecycle wire-up.
- [`tests/tee_attestation_test.rs`](tests/tee_attestation_test.rs) — integration test exercising the mock attestation flow end-to-end (config → provider init → attest call → response shape verification).

## Architectural note

The `HardwareTrust` trait was the most consequential design decision in the whole arc. By making it polymorphic from day 1 — not adding it as an afterthought — every later attestation provider (AMD SEV-SNP report verification, OP-TEE-backed RSA-2048 TA, ATECC608A secp256k1) implemented the same interface, and the UI/backend never needed a rewrite when the substrate changed. Provider-polymorphic DTO (`#[serde(tag = "provider")]`) followed naturally from this.

## Commits

`5474ff9f`, `0723ecd8`, `ff164535`, `964944cb`, `cf1934b5` (2026-02-15) — initial scaffold + mock providers + HTTP endpoints + integration test. `9c0990a0` (2026-02-16) — refactor hardcoded HTLC threshold into `TeeConfig` (C-009).
