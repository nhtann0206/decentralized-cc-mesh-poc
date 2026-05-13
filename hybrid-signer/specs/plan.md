# Implementation Plan: Confidential Compute Integration

**Branch**: `039-confidential-compute` | **Date**: 2026-02-14 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/039-confidential-compute/spec.md`
**Architecture Reference**: `.specify/memory/constitution.md` Principle IX (strictly followed for trait abstraction)
**Developer Guide**: For architecture details, see [contracts/ARCHITECTURE.md](contracts/ARCHITECTURE.md)

## Summary

Confidential Compute adds hardware-backed security to Lightning Node OS through **physical chip isolation** and **measured boot**. The implementation uses **trait abstraction** (HardwareTrust, VlsProvider) with two provider variants: **Mock** (development without hardware) and **Hardware** (ATECC608A+TPM via direct Radxa I2C for production). ~~ESP32-C6 gatekeeper has been **DROPPED**~~ — boot verification now handled by kernel-level TPM checks + systemd watchdog. A **hybrid signing strategy** balances security (ATECC608A cold path for commitment transactions) and performance (RAM hot path for HTLC forwards < 500k sats). The system is **feature-gated** (`TEE_ENABLED=false` by default) with zero impact on existing deployments. Debian hardening via read-only root filesystem and zRAM overlay provides stateless security. Integration follows the existing five-layer architecture with TEE providers injected via `AppState`.

## Technical Context

**Language/Version**: Rust 1.75+ (backend only — ~~ESP32 firmware DROPPED~~)
**Primary Dependencies**: i2cdev 0.6 (I2C), zeroize 1.7 (secure memory), bitflags 2.4, hkdf 0.12 + sha2 0.10 (key derivation)
**Storage**: No new database tables (TEE state is ephemeral or hardware-backed)
**Testing**: cargo test (backend integration tests with mock providers), CI/CD with `TEE_ENABLED=true TEE_PROVIDER=mock`
**Target Platform**: Linux (Debian Minimal on Radxa Zero 3W ARM64), macOS (development with mocks)
**Project Type**: Backend extension + infra scripts (Debian hardening). ~~ESP32 sub-project DROPPED.~~
**Performance Goals**: Boot attestation < 1s, ATECC608A signing < 100ms (p99), hot path signing < 5ms, 10 attestation requests/sec
**Constraints**: I2C 400kHz (hardware limit), 60s watchdog timeout (systemd WatchdogSec), Radxa 1GB RAM min (zRAM 256MB max, RAM optimization required)
**Scale/Scope**: ~10 new files (backend), 5 modified files, 3 infra scripts, 4 systemd services, Milestone 1-4 spans 6-8 weeks

## Constitution Check

*GATE: Must pass before Milestone 0 research. Re-check after Milestone 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Production-Ready Code | PASS | Mock providers are fully functional (non-placeholder). Hardware providers will be complete before Milestone 3 merge. |
| II. Type Safety | PASS | Enums for `TeeError`, `SigningStrategy`. Trait-based abstraction for providers. No free-form strings. |
| III. Performance | PASS | Async I/O (tokio-serial, tokio::process). I2C mutex prevents contention. Hybrid signing strategy optimizes latency. |
| IV. API Consistency | PASS | New `/api/internal/tee/attest` endpoint uses `ApiResponse<T>`. Standard error codes. |
| V. Testing | PASS | Integration test `tee_attestation_test.rs` covers mock Alice-Bob attestation. Hardware tests manual (requires physical setup). |
| VI. UX Consistency | N/A | Backend-only feature; no frontend UI in scope. |
| VII. Observability | PASS | All logging uses `target: "node_backend::tee"` with structured fields. NEVER logs private keys, nonces, signatures. |
| VIII. Security | PASS | `zeroize` crate for hot keys. ATECC608A private keys never exported. No secrets in logs. Feature-gated via env var. |
| IX. Layered Architecture | PASS (trait abstraction) | TEE providers integrate via trait injection (same pattern as Repository). No cross-layer violations. |
| X. Frontend Data Fetching | N/A | No frontend in scope. |
| XI. Shared API Client | N/A | No frontend in scope. |

**Gate Result**: PASS - All applicable principles satisfied. Trait abstraction aligns with existing Repository pattern (Constitution v2.2.0 Principle IX).

## Project Structure

### Documentation (this feature)

```text
specs/039-confidential-compute/
├── spec.md              # Feature specification (user scenarios, requirements, success criteria)
├── plan.md              # This file
├── README.md            # Feature overview and navigation
├── checklists/          # Progress tracking
│   ├── requirements.md  # Spec quality checklist (34/95 tracked)
│   └── tasks.md         # Task tracking (30 tasks)
└── contracts/           # Technical deep-dives
    ├── ARCHITECTURE.md  # Hardware topology, software stack
    ├── SECURITY_MODEL.md # Threat model and attack mitigations
    ├── DEMO_CHECKLIST.md # Demo readiness checklist
    └── DEPLOYMENT_GUIDE.md # Deployment instructions
```

### Source Code (repository root)

```text
packages/backend/src/
├── lib.rs                      # MODIFIED: add `pub mod tee;`
├── state.rs                    # MODIFIED: add `hardware_trust`, `vls_provider` fields to AppState
├── main.rs                     # MODIFIED: TEE initialization (feature-gated), auto_start attestation verification
├── tee/                        # NEW: 10 files (~1500 lines)
│   ├── mod.rs                  # Module exports, feature gate checks
│   ├── error.rs                # TeeError enum (UartError, I2cError, SigningError, ProtocolError, etc.)
│   ├── traits.rs               # HardwareTrust + VlsProvider trait definitions
│   ├── models.rs               # BootAttestation, AttestationChallenge, AttestationResponse, PlatformInfo, Signature
│   ├── signing_strategy.rs     # SigningStrategy enum (HotPath, ColdPath), threshold logic
│   ├── providers/
│   │   ├── mod.rs              # Re-exports, lazy_static I2C_MUTEX
│   │   ├── mock_hardware.rs    # MockHardwareTrust implementation (fake nonces, PCR values)
│   │   ├── mock_vls.rs         # MockVlsProvider (software ECDSA signing)
│   │   ├── esp32_hardware.rs   # ⛔ TO DROP (ESP32 DROPPED)
│   │   ├── tpm_hardware.rs     # TPM 2.0 I2C driver (PCR read, direct from Radxa)
│   │   └── atecc_vls.rs        # AteccVlsProvider (ATECC608A I2C signing, direct from Radxa)
│   ├── ldk_integration.rs      # VlsSignerProvider for LDK (Milestone 4)
│   └── tests/
│       ├── attestation_test.rs # Alice-Bob remote attestation (mock + hardware scenarios)
│       └── latency_benchmark.rs # I2C signing performance benchmarks
│
├── handlers/                   # MODIFIED: add TEE endpoints (or create transport/http_tee.rs)
│   └── tee_handlers.rs         # NEW: /api/internal/tee/attest, /api/internal/tee/heartbeat
│
└── transport/                  # ALTERNATIVE: if following new pattern
    └── node_server/
        └── http_tee.rs         # NEW: TEE HTTP handlers (protocol-specific)

⛔ esp32/src/                    # ENTIRE DIRECTORY DROPPED — ESP32-C6 gatekeeper removed from architecture

infra/qemu-tee/                 # Debian hardening scripts + QEMU simulation (~300 lines bash)
├── setup_readonly_rootfs.sh    # Read-only root /etc/fstab configuration
├── setup_zram_overlay.sh       # zRAM overlay setup (256MB)
├── provision_persistent.sh     # Format + mount persistent /var/lib/ldk partition
├── tee-watchdog.service        # Type=notify, WatchdogSec=60, sd_notify("WATCHDOG=1")
├── var-lib-ldk.mount           # /var/lib/ldk persistent mount unit
├── launch_qemu.sh              # Boot qemu-system-aarch64 with Debian ARM64 image
├── create_disk_images.sh       # Download Debian ARM64 cloud image + create persistent QCOW2
├── validate_all.sh             # Automated validation suite (RO root, zRAM, persistence, watchdog)
└── README.md                   # Prerequisites and usage instructions

docs/adr/                       # NEW: Architecture Decision Record
└── 005-confidential-compute.md # ADR documenting key architectural decisions
```

**Structure Decision**: Trait-based abstraction in new `tee/` module follows existing Repository pattern (Constitution Principle IX). ~~ESP32 firmware sub-project DROPPED~~ — all hardware communication now via direct I2C from Radxa. Debian hardening scripts live in `infra/qemu-tee/`. No database changes needed (TEE state is ephemeral or hardware-backed). Integration via `AppState` dependency injection.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| ~~Sub-project (ESP32 firmware)~~ | ⛔ **DROPPED** — ESP32-C6 gatekeeper removed. Boot verification now handled by kernel-level TPM checks + systemd watchdog. No separate firmware project needed. | — |
| Hybrid Signing Strategy | I2C latency (25-100ms) violates Lightning HTLC timing requirements (< 1s network propagation) | Always using ATECC608A would cause HTLC timeouts under load. Always using RAM keys would eliminate hardware isolation benefit. Hybrid approach is necessary trade-off. |

**Justification**: ~~Sub-project dropped — no longer a concern.~~ Hybrid signing is a **security vs performance trade-off** explicitly approved in PRD decisions (see CLAUDE.md Key Decisions table).

## Implementation Milestones

### Milestone 0: Research and Validation (Pre-Milestone 1)

**Goal**: Validate hardware components and toolchain setup.

**Research Questions**:
1. ~~Can ESP32-C6 control Radxa RESET pin via GPIO?~~ ⛔ **DROPPED** — ESP32 removed from architecture
2. What is actual ATECC608A I2C signing latency? (Benchmark: 25-100ms typical)
3. Does TPM 2.0 PCR-10 contain kernel hash in U-Boot? (Answer: Requires U-Boot patch)
4. Can Debian root be remounted read-only without breaking systemd? (Answer: Yes, with overlay)

**Deliverables**:
- Hardware BOM (Bill of Materials) finalized
- I2C bus topology (ATECC608A 0x60, TPM 0x2E — direct from Radxa)
- ~~ESP32 GPIO wiring diagram~~ ⛔ DROPPED
- ~~UART protocol specification~~ ⛔ DROPPED

**Output**: `research.md` documenting validation results and prototype findings.

---

### Milestone 1: Foundation (Trait Abstractions + Mock Providers) ✅ COMPLETE

**Goal**: Establish trait definitions and mock implementations for development without hardware.

**Duration**: Week 1-2

**Tasks**:

| Step | File(s) | Description |
|------|---------|-------------|
| 1.1 | `packages/backend/src/tee/traits.rs` | Define `HardwareTrust` trait (get_boot_attestation, read_tpm_pcr, send_heartbeat, attest) + `VlsProvider` trait (sign, sign_commitment, sign_htlc, sign_closing) |
| 1.2 | `packages/backend/src/tee/models.rs` | Define BootAttestation, AttestationChallenge, AttestationResponse, PlatformInfo structs with serde support |
| 1.3 | `packages/backend/src/tee/error.rs` | Define TeeError enum (UartError, I2cError, SigningError, ProtocolError, HardwareError) with `From` implementations |
| 1.4 | `packages/backend/src/tee/providers/mock_hardware.rs` | Implement MockHardwareTrust with fake RNG nonces, hardcoded PCR values, synthetic signatures |
| 1.5 | `packages/backend/src/tee/providers/mock_vls.rs` | Implement MockVlsProvider with software ECDSA signing (secp256k1 crate) |
| 1.6 | `packages/backend/src/state.rs` | Add `hardware_trust: Option<Arc<dyn HardwareTrust>>`, `vls_provider: Option<Arc<dyn VlsProvider>>` to AppState |
| 1.7 | `packages/backend/src/main.rs` | Add TEE initialization logic (read TEE_ENABLED, TEE_PROVIDER env vars, instantiate providers) |
| 1.8 | `packages/backend/src/tee/tests/attestation_test.rs` | Integration test: Alice (mock) generates attestation, Bob (mock) verifies (challenge-response) |
| 1.9 | `packages/backend/Cargo.toml` | Add dependencies: `tokio-serial = "5.4"`, `i2cdev = "0.6"`, `zeroize = "1.7"`, `bitflags = "2.4"` |
| 1.10 | Verify | `cargo check && cargo check --tests` - all code compiles. `cargo test tee_attestation_test` passes. |

**Deliverables**:
- `data-model.md` (if database changes, N/A for this feature)
- `quickstart.md` (developer guide for testing with mock providers)
- `contracts/tee-api.yaml` (OpenAPI spec for TEE endpoints, if exposed)

**Acceptance Criteria**:
- [ ] `TEE_ENABLED=false`: Node starts identically to current code
- [ ] `TEE_ENABLED=true TEE_PROVIDER=mock`: Mock providers initialize successfully
- [ ] Integration test passes: Alice-Bob attestation with mock providers
- [ ] CI/CD runs test suite with mock providers (no hardware dependencies)

---

### Milestone 2: Debian Hardening (Read-Only Root + zRAM Overlay) ✅ COMPLETE

**Goal**: Prepare Debian base image for production deployment with stateless security.

**Duration**: Week 2-3

**Tasks**:

| Step | File(s) | Description |
|------|---------|-------------|
| 2.1 | `infra/debian-minimal/setup_readonly_rootfs.sh` | Script to configure read-only root in /etc/fstab, backup original fstab |
| 2.2 | `infra/debian-minimal/setup_zram_overlay.sh` | Script to setup zRAM device (256MB), mount overlay for /etc, /var/log, /tmp |
| 2.3 | `infra/debian-minimal/provision_persistent.sh` | Format and mount persistent partition at `/var/lib/ldk`. Verify write → reboot → data survives. |
| 2.4 | `infra/debian-minimal/systemd/tee-watchdog.service` | `Type=notify`, `WatchdogSec=60`. Backend calls `sd_notify("WATCHDOG=1")` when healthy — no separate timer or curl needed. Systemd monitors backend process health (restart on missed sd_notify). ~~ESP32 UART watchdog layer DROPPED.~~ |
| 2.5 | `infra/qemu-tee/var-lib-ldk.mount` | Systemd mount unit for `/var/lib/ldk`. `RequiredBy=tee-watchdog.service`. Filename must match mount path per systemd convention. |
| 2.6 | Test on QEMU ARM64 | Deploy scripts to QEMU Debian ARM64 guest, verify read-only root, test zRAM overlay, verify /var/lib/ldk persistence. See `docs/conf-com/research/infrastructure/QEMU_SIMULATION_ASSESSMENT.md` for test matrix. |
| 2.7 | Documentation | Update `quickstart.md` with Debian hardening steps, troubleshooting guide |

**Deliverables**:
- Hardened Debian image for Radxa Zero 3W
- Systemd services installed and tested
- Documentation for manual deployment

**Acceptance Criteria**:
- [ ] Root filesystem mounts with `ro` flag after reboot
- [ ] Write to `/usr/bin/test` fails with "Read-only file system"
- [ ] Write to `/etc/test` succeeds (zRAM overlay)
- [ ] After reboot, `/etc/test` is gone (ephemeral)
- [ ] `/var/lib/ldk` persists across reboots
- [ ] Backend service reports WATCHDOG=1 via sd_notify within WatchdogSec interval
- [ ] Systemd restarts backend if sd_notify heartbeat missed

---

### Milestone 3: Hardware Integration (Direct Radxa I2C → ATECC608A + TPM)

**Goal**: Implement real hardware providers for production deployment. ~~ESP32-C6 gatekeeper DROPPED~~ — Radxa communicates directly with ATECC608A and TPM via I2C.

**Duration**: Week 4-6

**Tasks**:

> ⛔ **ESP32 tasks (3.1-3.7) DROPPED** — Replaced with direct Radxa↔I2C provider tasks below.

| Step | File(s) | Description |
|------|---------|-------------|
| 3.1 | `packages/backend/src/tee/providers/tpm_hardware.rs` | Implement `LinuxI2cHandler` for TPM 2.0: PCR-10 read via `/dev/i2c-*` (addr 0x2E). Use `i2cdev` crate. |
| 3.2 | `packages/backend/src/tee/providers/atecc_vls.rs` | Implement `AteccVlsProvider`: ATECC608A SIGN command (OpCode 0x41) via direct I2C (addr 0x60). I2C_MUTEX for bus contention prevention. |
| 3.3 | `packages/backend/src/tee/providers/radxa_hardware.rs` | Implement `RadxaHardwareTrust` (replaces `Esp32HardwareTrust`): reads TPM PCR-10 via I2C, verifies kernel hash against known-good baseline. No UART needed. |
| 3.4 | Boot verification logic | Kernel-level TPM verification flow: U-Boot extends PCR-10 → backend reads PCR-10 → compares against stored hash → mismatch triggers recovery mode. |
| 3.5 | `packages/backend/src/tee/providers/mod.rs` | Wire `RadxaHardwareTrust` + `AteccVlsProvider` into provider factory. `TEE_PROVIDER=hardware` instantiates direct I2C providers. |
| 3.6 | QEMU `i2c-stub` validation | Test `LinuxI2cHandler` against QEMU ARM64 with `i2c-stub` kernel module before deploying to real hardware. |
| 3.7 | Deploy to Radxa | Flash hardened Debian image, connect ATECC608A + TPM via I2C bus, verify communication. |
| 3.8 | Integration test | Alice (hardware, Radxa) ↔ Bob (mock) attestation. Verify `is_hardware_backed: true`. |
| 3.9 | Benchmark | Run `latency_benchmark.rs`: ATECC608A signing < 100ms (p99), TPM PCR read < 50ms, hot path < 5ms. |
| 3.10 | Cleanup | Remove `esp32_hardware.rs`, `cobs.rs` (ESP32 artifacts). Delete `esp32/` directory if present. |

**Deliverables**:
- ~~ESP32 firmware~~ ⛔ DROPPED — Direct I2C providers from Radxa
- Hardware providers (ATECC608A + TPM) tested on Radxa
- Latency benchmarks documented

**Acceptance Criteria**:
- [ ] ATECC608A signing completes within 100ms (p99) via direct I2C
- [ ] TPM PCR-10 read returns kernel hash via direct I2C
- [ ] Alice (hardware) ↔ Bob (mock) attestation successful
- [ ] Systemd watchdog restarts backend on missed sd_notify

---

### Milestone 4: The Vault (VLS Hybrid Signing & Seed Encryption)

**Goal**: Integrate with LDK for Lightning operations using hybrid hot/cold signing, and encrypt the LDK seed at rest.

**Duration**: Week 6-8

**Tasks**:

| Step | File(s) | Status | Description |
|------|---------|--------|-------------|
| 4.1 | `ldk-node` fork: `src/wallet/mod.rs`, `src/types.rs`, `src/builder.rs`, `src/lib.rs` | ✅ Phase 1 | `HybridSigner` enum + `TeeSignerFactory` trait injection. `dyn SignerProvider` impossible due to LDK compile-time associated type — enum dispatch instead. |
| 4.2 | `packages/backend/src/tee/signing_strategy.rs` | Pending | SigningStrategy enum, select_strategy() logic (amount threshold 500k sats) |
| 4.3 | `packages/backend/src/tee/ldk_integration.rs` | Pending | Implement `TeeSignerFactory` in backend, connect to ATECC608A via I2C actor |
| 4.4 | Fork Phase 2 | Pending | Replace `Tee(InMemorySigner)` with `Tee(Atecc608aSigner)`, wrap I2C in `spawn_blocking` |
| 4.5 | `packages/backend/src/tee/seed_encryption.rs` | Pending | ATECC608A wrapping key derivation to encrypt `signer_seed.hex` at rest via AES-256-GCM |
| 4.6 | Test Lightning | Pending | Open channel Alice → Bob, forward HTLC (trigger hot path), close channel (trigger cold path) |
| 4.7 | Benchmark | Pending | Measure HTLC forward latency (< 5ms), commitment signing latency (< 100ms) |
| 4.8 | Documentation | Pending | Update `quickstart.md` with Lightning integration guide |

**Key discoveries (Step 4.1)**:
- LDK does NOT serialize signers (writes `0u32` since v0.0.113) — zero backward compat risk
- `read_chan_signer` is legacy-only (pre-v0.0.113). Modern channels always use `derive_channel_signer()`
- Only 2 direct `InMemorySigner` references needed changing; 12 `Arc<KeysManager>` auto-resolve
- Third-party "Reboot Bypass" audit claim cross-validated as FALSE ALARM

**Deliverables**:
- ✅ HybridSigner fork injection (Phase 1 — type system validated, cargo check passes)
- Pending: Backend `TeeSignerFactory` implementation connecting to ATECC608A
- Pending: Lightning channel operations tested
- Pending: Performance benchmarks documented

**Acceptance Criteria**:
- [x] `cargo check` passes with HybridSigner (0 errors from our code)
- [x] `TeeSignerFactory` injectable via `NodeBuilder::set_tee_signer_factory()`
- [x] `HybridSigner` implements `ChannelSigner` + `EcdsaChannelSigner` + `Clone` + `Writeable`
- [ ] Phase 2: Replace `Tee(InMemorySigner)` with `Tee(Atecc608aSigner)` — requires hardware
- [ ] Phase 2: Wrap I2C calls in `spawn_blocking` (EcdsaChannelSigner is sync)
- [ ] Channel opens successfully (commitment signed by ATECC608A)
- [ ] HTLC < 500k sats signed via hot path (< 5ms)
- [ ] HTLC > 500k sats signed via cold path (< 100ms)
- [ ] Channel closes successfully (closing tx signed by ATECC608A)
- [ ] No Lightning errors or HTLC timeouts

---

### Milestone 5: The Gateway & The Sandbox (L402 & Agent Security)

**Goal**: Secure the L402 Macaroon root key using ATECC608A and decouple the AgentExecutor from LDK memory.

**Duration**: Week 8

**Tasks**:

| Step | File(s) | Description |
|------|---------|-------------|
| 5.1 | `packages/backend/src/l402/repository.rs` | Derive ATECC608A wrapping key to encrypt `macaroon_root_key` via AES-256-GCM before SQLite persistence |
| 5.2 | `packages/backend/src/l402/middleware.rs` | Decrypt root key on boot into `SecureMemory` (RAM, mlock) for high-performance Macaroon validation |
| 5.3 | `packages/backend/src/ai_agents/executor.rs` | ✅ COMPLETE — Removed `Arc<ldk_node::Node>` dependency to break Prompt Injection → RCE → LDK Memory Dump attack chain |
| 5.4 | Test L402 | Verify Macaroon continuity across crashes/reboots to ensure wrapping key derivation is stable |

**Deliverables**:
- L402 Macaroon key encrypted at rest
- AgentExecutor isolated from LDK
- Continuity test suites

**Acceptance Criteria**:
- [ ] SQLite `lightning.db` contains no plaintext `macaroon_root_key`
- [ ] Valid Macaroons survive a full system reboot (wrapping key recovery)
- [x] AgentExecutor compiles and functions without `Arc<ldk_node::Node>` — ✅ M5.3 COMPLETE

---

### Milestone 6: Documentation and Cleanup

**Goal**: Finalize documentation, create ADR-005, prepare for merge.

**Duration**: Week 9

**Tasks**:

| Step | File(s) | Description |
|------|---------|-------------|
| 6.1 | `docs/adr/005-confidential-compute.md` | Create ADR documenting architectural decisions (Debian vs Alpine, Hybrid signing, etc.) |
| 6.2 | `specs/039-confidential-compute/checklists/requirements.md` | Create checklist verifying all spec requirements met |
| 6.3 | `CLAUDE.md` | Update with Milestone 1-5 completion status, production deployment notes |
| 6.4 | `specs/039-confidential-compute/quickstart.md` | Finalize developer guide (mock setup, hardware provisioning, troubleshooting) |
| 6.5 | Code review | Self-review for Constitution compliance, security audit (no secrets in logs), performance verification |
| 6.6 | CI/CD | Add `cargo audit` to GitHub Actions pipeline for dependency scanning |
| 6.7 | Merge preparation | Squash commits, write comprehensive PR description, tag reviewers |

**Deliverables**:
- ADR-005 published
- Complete documentation set
- PR ready for review

**Acceptance Criteria**:
- [ ] All spec requirements (FR-001 to FR-020) met
- [ ] All success criteria (SC-001 to SC-015) verified
- [ ] Constitution check passes (all applicable principles)
- [ ] CI/CD green (mock tests pass)
- [ ] Hardware testing documented (manual verification)

---

## Post-Milestone Enhancements (Future Work)

### Milestone 7: Production Hardening (Week 9+)

**Out of Scope for Initial Release**:

1. **Encrypted I2C** (Week 10-11): Enable ATECC608A encrypted I2C mode with session keys
2. **Transparency Log** (Week 11-12): Log firmware updates to public transparency log (Rekor)
3. **Database Encryption** (Week 12+): Full SQLite encryption for channel state and user data using ATECC608A wrapping key
4. **WASM AI Sandbox** (Week 13+): Full process isolation/WASM sandbox for AI execution
5. **Hardware Packaging** (Week 14+): Tamper-evident casing, epoxy coating
6. **Certification** (6-12 months): FIPS 140-3 Level 2, Common Criteria EAL4+

---

## Risk Mitigation

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Hardware supply chain delays | Medium | High | Order hardware early (Milestone 0), use breadboard prototypes for Milestone 3 |
| I2C latency exceeds 100ms | Low | High | Benchmark early (Milestone 3.12), implement async signing queue if needed |
| LDK API incompatibility | Low | Medium | Review LDK SignerProvider trait early (Milestone 4.1), engage with LDK team |
| Debian RO root breaks systemd | Medium | Medium | Test thoroughly on Radxa (Milestone 2.6), upstream fixes if needed |
| ~~ESP32 firmware bugs~~ | ⛔ | DROPPED | ESP32-C6 gatekeeper removed from architecture |
| ATECC608A provisioning errors | Low | Critical | Document provisioning carefully, test on dev boards first |

---

## Dependencies

**Milestone 1 → Milestone 2**: Independent (can run in parallel)
**Milestone 2 → Milestone 3**: Hardened image required for hardware testing
**Milestone 3 → Milestone 4**: Hardware providers must work before LDK integration
**Milestone 4 → Milestone 5**: ATECC608A wrapping key (M4) required for L402 encryption (M5)
**Milestone 5 → Milestone 6**: Documentation depends on all features complete

**External Dependencies**:
- ~~ESP32-C6 development board~~ ⛔ DROPPED
- ATECC608A breakout board (order during Milestone 0)
- TPM 2.0 module (order during Milestone 0)
- Radxa Zero 3W (already available)

---

**See Also**:
- [spec.md](spec.md) - User scenarios, requirements, acceptance criteria
- [ARCHITECTURE.md](contracts/ARCHITECTURE.md) - Hardware topology, software stack, integration patterns
- [SECURITY_MODEL.md](contracts/SECURITY_MODEL.md) - Threat model, attack scenarios, mitigations
