# TEE Foundation Module

## What this is

The Confidential Compute substrate of the backend. Before this existed, the codebase had no concept of hardware-rooted attestation at all. After this, every later CC piece — the AMD SEV-SNP verifier on cloud, the OP-TEE Trusted Application on ARM, the hardware-backed Lightning signer — plugged into the same set of interfaces and the rest of the system didn't have to care which substrate was in play.

## What it does

Exposes two polymorphic abstractions to the rest of the backend:

- **`HardwareTrust`** — "attest a measurement against a hardware-rooted key." One trait, many implementations (mock for CI, AMD SEV-SNP, ARM OP-TEE TA, secure element). The rental flow, the agent system, and the UI all call this trait — they never know which hardware is underneath.
- **`VlsProvider`** — the Validating Lightning Signer interface that gates HTLC signing on policy (channel-state-machine correctness, HTLC value threshold). This is the trust boundary where the project decides "this Lightning state transition is safe to sign."

It also ships the HTTP surface (`/api/tee/attest`, `/api/tee/status`, `/api/tee/circuit-breaker`) that a renter's browser calls when it wants a fresh attestation report from the operator, plus typed config, typed errors, mock implementations for CI/dev, and an integration test exercising the full attestation flow.

## Why this design choice mattered

Polymorphism was decided up front, not bolted on later. That is the only reason the same renter-facing rental UI worked unchanged when the backing hardware moved from cloud AMD to ARM TrustZone — and would work unchanged for a future RISC-V Keystone or Apple Silicon target. Most attestation codebases treat the chip family as a hardcoded assumption and pay for that decision indefinitely; this one doesn't.

## Key files

- [`src/traits.rs`](src/traits.rs) — the two trait definitions. The design contract.
- [`src/http_tee.rs`](src/http_tee.rs) — the attestation HTTP endpoints.
- [`src/providers/mock_hardware.rs`](src/providers/mock_hardware.rs), [`mock_vls.rs`](src/providers/mock_vls.rs) — deterministic mocks for CI.
- [`tests/tee_attestation_test.rs`](tests/tee_attestation_test.rs) — end-to-end integration test.
