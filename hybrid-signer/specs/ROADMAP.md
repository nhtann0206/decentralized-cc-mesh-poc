# Implementation Roadmap

> **Relationship to plan.md**: [plan.md](plan.md) is the authoritative implementation plan (task assignments, acceptance criteria, constitution check, risk mitigation). This ROADMAP.md is the **developer-facing guide** with code templates, step-by-step instructions, and quick start recipes. For milestone status and task ownership, refer to plan.md.

## Quick Start Guide

### Starting Implementation

**First file to create**: `packages/backend/src/tee/mod.rs`

**First commands**:
```bash
# Create module structure (5 minutes)
mkdir -p packages/backend/src/tee/{providers,tests}
touch packages/backend/src/tee/{mod.rs,error.rs,config.rs,traits.rs}
touch packages/backend/src/tee/providers/{mod.rs,mock_hardware.rs,mock_vls.rs}
touch packages/backend/src/tee/tests/attestation_test.rs

# Verify directory structure
tree packages/backend/src/tee
```

**Estimated Milestone 1 completion**: 10-15 hours (can be split over 3-5 days)

**Next steps**: Follow Milestone 1.0 → 1.5 below in order.

---

## Timeline Summary

| Milestone | Duration | Goal | Status |
|-----------|----------|------|--------|
| **Milestone 1** | Week 1-2 | Trait abstractions + mocks | ✅ COMPLETE |
| **Milestone 2** | Week 2-3 | Debian hardening | ✅ COMPLETE |
| **Milestone 3** | Week 4-6 | Hardware integration (direct I2C) | ⏳ Blocked on hardware |
| **Milestone 4** | Week 6-8 | VLS hybrid signing | ✅ Phase 1 (HybridSigner fork) |
| **Milestone 5** | Week 8 | L402 + Agent isolation | ✅ M5.3 (AgentExecutor) |

**Total Duration**: 6-8 weeks for core functionality. **ESP32-C6 DROPPED** — see [plan.md](plan.md) for details.

## Milestone 1: Foundation - Mock Everything ✅ COMPLETE

**Duration**: Week 1-2 (10-15 hours)
**Goal**: Establish trait abstractions and mock implementations for development

### Objectives

✅ **Developer Experience**: Build/test TEE features without hardware, CI/CD without dependencies
✅ **Architectural Foundation**: Trait definitions, error handling, logging conventions

### Task Breakdown

| Step | Duration | Files | Output | Checklist |
|------|----------|-------|--------|-----------|
| **1.0: Module Setup** | 30min | `mod.rs`, directory structure | Module skeleton | [ ] Directory created<br>[ ] `mod.rs` exports defined<br>[ ] `cargo check` passes |
| **1.1: Error & Config** | 1h | `error.rs`, `config.rs` | Error types, env parsing | [ ] `TeeError` enum defined<br>[ ] `TeeConfig::from_env()` works<br>[ ] Unit tests pass |
| **1.2: Trait Definitions** | 2h | `traits.rs` | HardwareTrust + VlsProvider | [ ] Traits compile<br>[ ] Data structures defined<br>[ ] Rustdoc comments added |
| **1.3: Mock Implementations** | 2-3h | `providers/mock_*.rs` | Mock providers + tests | [ ] MockHardwareTrust impl<br>[ ] MockVlsProvider impl<br>[ ] `is_hardware_backed: false` |
| **1.4: AppState Integration** | 1-2h | `app_state.rs`, `main.rs` | Backend initialization | [ ] TEE providers in AppState<br>[ ] `TEE_ENABLED=false` works<br>[ ] Mock initialization works |
| **1.5: Integration Tests** | 2h | `tests/attestation_test.rs` | Alice ↔ Bob test | [ ] Mock attestation test passes<br>[ ] Test runs in < 100ms<br>[ ] CI integration complete |
| **1.6: API Endpoints** | 2-3h (optional) | `routes/tee.rs`, `services/` | Attestation endpoint | [ ] POST /api/internal/tee/attest<br>[ ] Returns mock data<br>[ ] Error handling tested |

**Total**: 10-15 hours (can parallelize 1.1-1.2, or 1.3-1.4)

### Detailed Step-by-Step

<details>
<summary><strong>Step 1.0: Module Setup</strong> (30 minutes)</summary>

**Files to create**:
```
packages/backend/src/tee/
├── mod.rs
├── error.rs
├── config.rs
├── traits.rs
└── providers/
    ├── mod.rs
    ├── mock_hardware.rs
    └── mock_vls.rs
```

**mod.rs template**:
```rust
//! Trusted Execution Environment (TEE) module
pub mod config;
pub mod error;
pub mod traits;
pub mod providers;

pub use config::TeeConfig;
pub use error::TeeError;
pub use traits::{HardwareTrust, VlsProvider};
```

**Update `packages/backend/src/lib.rs`**:
```rust
pub mod tee;
```

**Verify**: `cargo check` should pass.
</details>

<details>
<summary><strong>Step 1.1: Error Types & Config</strong> (1 hour)</summary>

**error.rs**:
```rust
#[derive(Debug, thiserror::Error)]
pub enum TeeError {
    #[error("TEE not enabled")]
    NotEnabled,
    #[error("Hardware error: {0}")]
    HardwareError(String),
    #[error("Attestation failed: {0}")]
    AttestationFailed(String),
    #[error("Signing error: {0}")]
    SigningError(String),
}
```

**config.rs**:
```rust
#[derive(Clone, Debug)]
pub struct TeeConfig {
    pub enabled: bool,
    pub provider: TeeProviderType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TeeProviderType {
    Mock,
    Hardware { i2c: String, atecc_addr: u8 },  // ESP32 UART dropped — direct I2C only
}

impl TeeConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("TEE_ENABLED")
            .unwrap_or_else(|_| "false".to_string()) == "true";

        let provider = match std::env::var("TEE_PROVIDER").as_deref() {
            Ok("hardware") => TeeProviderType::Hardware {
                i2c: std::env::var("TEE_TPM_I2C").unwrap_or_else(|_| "/dev/i2c-1".into()),
                atecc_addr: std::env::var("TEE_ATECC_ADDR")
                    .ok().and_then(|s| u8::from_str_radix(&s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0x60),
            },
            _ => TeeProviderType::Mock,
        };

        Self { enabled, provider }
    }
}
```
</details>

<details>
<summary><strong>Step 1.2: Trait Definitions</strong> (2 hours)</summary>

**traits.rs** (outline - see ARCHITECTURE.md for full):
```rust
use async_trait::async_trait;
use std::time::SystemTime;

// Data structures
pub struct BootAttestation {
    pub nonce: [u8; 32],
    pub kernel_hash: [u8; 32],
    pub signature: [u8; 64],
    pub verified_at: SystemTime,
    pub is_hardware_backed: bool,
}

pub struct AttestationChallenge {
    pub requester_node_id: String,
    pub challenge_nonce: [u8; 32],
    pub timestamp: SystemTime,
}

pub struct AttestationResponse {
    pub boot_measurement: BootAttestation,
    pub platform_info: PlatformInfo,
}

// Traits
#[async_trait]
pub trait HardwareTrust: Send + Sync {
    async fn get_boot_attestation(&self) -> Result<BootAttestation, TeeError>;
    async fn read_tpm_pcr(&self, pcr: u8) -> Result<[u8; 32], TeeError>;
    async fn send_heartbeat(&self) -> Result<(), TeeError>;
    async fn attest(&self, challenge: AttestationChallenge) -> Result<AttestationResponse, TeeError>;
}

#[async_trait]
pub trait VlsProvider: Send + Sync {
    async fn sign(&self, data: &[u8]) -> Result<Signature, TeeError>;
    async fn sign_commitment(&self, tx: &CommitmentTx) -> Result<Signature, TeeError>;
    async fn sign_htlc(&self, htlc: &HtlcData) -> Result<Signature, TeeError>;
}
```
</details>

<details>
<summary><strong>Step 1.3: Mock Implementations</strong> (2-3 hours)</summary>

**providers/mock_hardware.rs**:
```rust
pub struct MockHardwareTrust {
    fake_nonce: [u8; 32],
    fake_pcr: HashMap<u8, [u8; 32]>,
}

impl MockHardwareTrust {
    pub fn new() -> Self {
        // Generate realistic fake data
        // ...
    }
}

#[async_trait]
impl HardwareTrust for MockHardwareTrust {
    async fn get_boot_attestation(&self) -> Result<BootAttestation, TeeError> {
        Ok(BootAttestation {
            nonce: self.fake_nonce,
            kernel_hash: self.fake_pcr[&10],
            signature: [0x42; 64],
            verified_at: SystemTime::now(),
            is_hardware_backed: false,  // CRITICAL: Mock = false
        })
    }
    // ... other methods
}
```

**Key requirement**: `is_hardware_backed: false` for all mock providers.
</details>

<details>
<summary><strong>Step 1.4: AppState Integration</strong> (1-2 hours)</summary>

**app_state.rs**:
```rust
pub struct AppState {
    // ... existing fields
    pub tee_trust: Option<Arc<dyn HardwareTrust>>,
    pub tee_vls: Option<Arc<dyn VlsProvider>>,
}
```

**main.rs**:
```rust
let config = TeeConfig::from_env();
let (trust, vls) = if config.enabled {
    tee::initialize_providers(config).await?
} else {
    (None, None)
};

let app_state = AppState {
    // ... existing
    tee_trust: trust,
    tee_vls: vls,
};
```
</details>

### Acceptance Criteria (Milestone 1)

- [ ] `cargo check` passes without TEE enabled
- [ ] `TEE_ENABLED=false cargo run` → backend starts normally
- [ ] `TEE_ENABLED=true TEE_PROVIDER=mock cargo run` → backend starts with mock providers
- [ ] Mock attestation returns `is_hardware_backed: false`
- [ ] Integration test: Alice (mock) ↔ Bob (mock) attestation works
- [ ] CI/CD runs both modes without errors

### Deliverables

- ✅ Module structure created
- ✅ Traits defined and documented
- ✅ Mock implementations functional
- ✅ Integration tests passing
- ✅ CI/CD testing both TEE modes

### Dependencies

**None** (Milestone 1 is self-contained)

**Hardware Requirements**: None (mock-only)

---

---

## Milestone 2: Debian Hardening ✅ COMPLETE

**Duration**: Week 2-3
**Goal**: Prepare base OS for production deployment with read-only root and zRAM overlay

### Task Breakdown

| Step | Duration | Files/Scripts | Output |
|------|----------|---------------|--------|
| **2.1: Read-Only RootFS** | 2-3h | `infra/debian-minimal/setup_readonly_rootfs.sh` | RO root configured |
| **2.2: zRAM Overlay** | 2h | `infra/debian-minimal/setup_zram_overlay.sh` | 256MB ephemeral overlay |
| **2.3: Systemd Services** | 2h | `systemd/tee-watchdog.{service,timer}` | 30s heartbeat timer |
| **2.4: Radxa Testing** | 4-6h | Hardware validation | Tested on Radxa Zero 3W |

**Total**: ~10-13 hours

### Objectives

✅ **Stateless Security**: RO root, RAM overlay, persistent `/var/lib/ldk`
✅ **Production Readiness**: Systemd services, tested on Radxa hardware

### Detailed Tasks

<details>
<summary><strong>Step 2.1: Read-Only Root Filesystem</strong> (2-3 hours)</summary>

**File**: `infra/debian-minimal/setup_readonly_rootfs.sh`

**Key changes to `/etc/fstab`**:
```bash
# Read-only root
/dev/mmcblk0p2  /               ext4    ro,noatime              0 1

# Writable tmpfs (RAM-backed, ephemeral)
tmpfs           /run            tmpfs   defaults,size=64M       0 0
tmpfs           /var/log        tmpfs   defaults,size=32M       0 0
tmpfs           /tmp            tmpfs   defaults,size=128M      0 0

# Persistent Lightning data (survives reboot)
/dev/mmcblk0p3  /var/lib/ldk    ext4    defaults,noatime        0 2
```

**Acceptance**:
- [ ] `mount | grep "on / "` shows `ro` flag
- [ ] Write to `/usr/bin` fails with "Read-only file system"
- [ ] `/var/lib/ldk` persists across reboots
</details>

<details>
<summary><strong>Step 2.2: zRAM Overlay</strong> (2 hours)</summary>

**File**: `infra/debian-minimal/setup_zram_overlay.sh`

**Creates**: 256MB zRAM device for ephemeral writes (compressed → ~500MB effective)

**Acceptance**:
- [ ] `df -h | grep zram0` shows 256MB
- [ ] Write to `/etc/test` succeeds
- [ ] After reboot, `/etc/test` is gone
</details>

<details>
<summary><strong>Step 2.3: Systemd Watchdog Service</strong> (2 hours)</summary>

**Files**: `systemd/tee-watchdog.{service,timer}`

**Timer config**: Activates 30s after boot, repeats every 30s

**Acceptance**:
- [ ] Timer activates 30s after boot
- [ ] Heartbeat endpoint called every 30s
- [ ] Logs in `journalctl -u tee-watchdog`
</details>

<details>
<summary><strong>Step 2.4: Radxa Hardware Testing</strong> (4-6 hours)</summary>

**Hardware**: Radxa Zero 3W + MicroSD (16GB+, partitioned)

**Test sequence**:
1. Flash Debian Minimal → Run `setup_readonly_rootfs.sh` → Reboot
2. Verify RO root: `touch /usr/bin/test` (should fail)
3. Run `setup_zram_overlay.sh`
4. Verify overlay: `touch /etc/test` (should succeed)
5. Reboot → verify `/etc/test` gone
6. Create `/var/lib/ldk/test.txt` → Reboot → verify persistence

**Acceptance**: All tests pass on Radxa hardware, boot < 30s
</details>

### Acceptance Criteria (Milestone 2)

- [ ] Root filesystem mounts read-only
- [ ] zRAM overlay handles writable directories (ephemeral)
- [ ] `/var/lib/ldk` persists across reboots
- [ ] Systemd watchdog service activates and logs heartbeats
- [ ] All tests pass on Radxa Zero 3W hardware

### Dependencies

- Radxa Zero 3W hardware
- Debian Minimal image (existing infra)
- MicroSD card (16GB+, partitioned)

---

---

## Milestone 3: Hardware Integration

**Duration**: Week 4-6
**Goal**: Implement real hardware providers for ATECC608A and TPM via direct Radxa I2C. ~~ESP32-C6 DROPPED.~~

### Task Breakdown

| Step | Duration | Component | Output |
|------|----------|-----------|--------|
| **3.1: Direct I2C Hardware Providers** | 1 week | `tpm_hardware.rs`, `atecc_vls.rs` | Direct Radxa↔chip I2C communication |
| ~~**3.2: ESP32 Firmware**~~ | ⛔ DROPPED | ~~`esp32_hardware.rs`~~ | ~~ESP32 UART communication~~ |
| **3.3: ATECC608A VLS** | 3-4 days | `atecc_vls.rs` | I2C signing (< 100ms) |
| **3.4: Integration Testing** | 2-3 days | Alice (hardware) ↔ Bob (mock) | Mixed deployment test |

> **Architecture Change**: ESP32-C6 gatekeeper has been **DROPPED**. Radxa communicates directly with ATECC608A and TPM via I2C. Boot verification handled by kernel-level TPM checks + systemd watchdog. See `docs/conf-com/research/tee/architecture_synthesis_phase3.md` for details.

**Total**: 3-4 weeks

### Objectives

✅ **Hardware Communication**: Direct I2C from Radxa to ATECC608A + TPM. ~~ESP32 UART DROPPED.~~
✅ **Mixed Deployments**: Alice (hardware) ↔ Bob (mock) attestation

### Detailed Tasks

<details>
<summary><strong>Step 3.1: Direct I2C Hardware Providers</strong> (1 week)</summary>

> ⛔ **ESP32-C6 Firmware DROPPED** — No separate gatekeeper firmware. Radxa communicates directly with chips via I2C.

**Files**: `tpm_hardware.rs`, `atecc_vls.rs`

**Key tasks**:
- Implement `LinuxI2cHandler` for direct `/dev/i2c-*` access
- TPM 2.0 PCR-10 read via I2C (addr 0x2E)
- ATECC608A SIGN command via I2C (addr 0x60)
- I2C mutex to prevent bus contention between TPM and ATECC608A

**Boot Verification Flow (post-ESP32)**:
```
1. U-Boot measures kernel → extends TPM PCR-10
2. Backend reads TPM PCR-10 via I2C
3. Backend verifies hash against known-good value
4. Mismatch → log alert + enter recovery mode
```

**Acceptance**:
- [ ] ATECC608A signs test vector via direct I2C
- [ ] TPM PCR-10 readable via direct I2C
- [ ] I2C mutex prevents bus contention
- [ ] Systemd watchdog (WatchdogSec=60) functional
</details>

<details>
<summary><strong>~~Step 3.2: ESP32 Rust Provider~~</strong> ⛔ DROPPED</summary>

> ESP32-C6 gatekeeper removed from architecture. `esp32_hardware.rs` to be deleted. All hardware communication now via direct I2C providers in Step 3.1 and 3.3.

</details>

<details>
<summary><strong>Step 3.3: ATECC608A VLS Provider</strong> (3-4 days)</summary>

**File**: `atecc_vls.rs`

**Key features**:
- I2C mutex to prevent TPM/ATECC contention
- SIGN command (OpCode 0x41)
- Latency logging (`node_backend::tee`)

**Acceptance**:
- [ ] I2C communication successful (0x60)
- [ ] Signing latency < 100ms
- [ ] Signature ECDSA-verifiable
- [ ] I2C mutex prevents contention
</details>

<details>
<summary><strong>Step 3.4: Integration Testing</strong> (2-3 days)</summary>

**Test**: Alice (Radxa hardware) ↔ Bob (mock)

```bash
# Alice (hardware — direct I2C, no ESP32)
TEE_ENABLED=true TEE_PROVIDER=hardware \
TEE_TPM_I2C=/dev/i2c-1 TEE_ATECC_ADDR=0x60 \
cargo run -- --env alice.env

# Bob (mock)
TEE_ENABLED=true TEE_PROVIDER=mock cargo run -- --env bob.env
```

**Acceptance**:
- [ ] Alice: `is_hardware_backed: true`
- [ ] Bob accepts Alice's attestation
- [ ] Mixed deployment works
</details>

### Acceptance Criteria (Milestone 3)

- [ ] Direct I2C providers functional (ATECC608A + TPM from Radxa)
- [ ] Hardware providers tested on Radxa
- [ ] Alice (hardware) ↔ Bob (mock) attestation working
- [ ] ATECC608A signing < 100ms latency
- [ ] Systemd watchdog restarts backend on missed sd_notify
- [ ] Documentation: Hardware assembly guide

### Dependencies

**Hardware**:
- ~~ESP32-C6 dev board~~ ⛔ DROPPED
- ATECC608A breakout (Adafruit)
- TPM 2.0 module (Infineon SLB9670)
- Radxa Zero 3W with I2C pins accessible

---

---

## Milestone 4: VLS Hybrid Signing

**Duration**: Week 6-8
**Goal**: Integrate with LDK for Lightning operations using hybrid hot/cold signing

### Task Breakdown

| Step | Duration | Component | Output |
|------|----------|-----------|--------|
| **4.1: LDK SignerProvider** | 1 week | `ldk_integration.rs` | Custom signer integration |
| **4.2: Latency Benchmarking** | 2-3 days | `benchmarks/latency_benchmark.rs` | < 100ms cold, < 5ms hot |
| **4.3: Lightning Testing** | 3-4 days | End-to-end channel ops | Open/forward/close tests |

**Total**: 2-3 weeks

### Objectives

✅ **Lightning Integration**: Custom VLS signer, hot/cold path selection
✅ **Performance Validation**: Benchmark latency, verify no HTLC timeouts

### Detailed Tasks

<details>
<summary><strong>Step 4.1: LDK Fork — HybridSigner Integration</strong> (1 week)</summary>

**Status**: Phase 1 COMPLETE (type system validated, cargo check passes)

**Architecture Decision**: `dyn SignerProvider` is impossible due to LDK's compile-time associated type system (`type EcdsaSigner: EcdsaChannelSigner`). Solution: `HybridSigner` enum with runtime dispatch.

**Files changed in `our LDK fork` fork (branch `039-tee-hybrid-signer`)**:
- `src/wallet/mod.rs`: +`HybridSigner` enum, +`TeeSignerFactory` trait, `WalletKeysManager::EcdsaSigner = HybridSigner`
- `src/types.rs`: `ChainMonitor<HybridSigner>` (was `InMemorySigner`)
- `src/builder.rs`: `NodeBuilder::set_tee_signer_factory()` injection point
- `src/lib.rs`: `pub use wallet::{HybridSigner, TeeSignerFactory}`

**Key implementation**:
```rust
// Fork: src/wallet/mod.rs
pub enum HybridSigner {
    Software(InMemorySigner),  // Default — identical to upstream
    Tee(InMemorySigner),       // Mock Phase 1 / ATECC608A Phase 2
}

pub trait TeeSignerFactory: Send + Sync {
    fn derive_channel_signer(
        &self, channel_value_satoshis: u64, channel_keys_id: [u8; 32],
        software_signer: InMemorySigner, // Fallback for mock
    ) -> HybridSigner;
}

// WalletKeysManager::SignerProvider impl:
fn derive_channel_signer(&self, cv: u64, cki: [u8; 32]) -> HybridSigner {
    let sw = self.inner.derive_channel_signer(cv, cki);
    match &self.tee_signer_factory {
        Some(f) => f.derive_channel_signer(cv, cki, sw),
        None => HybridSigner::Software(sw),
    }
}
```

**Key discoveries**:
- LDK does NOT serialize signers to disk (writes `0u32` since v0.0.113). Zero backward compat risk.
- 12 `Arc<KeysManager>` references in `types.rs` auto-resolve. Only 2 lines changed directly.
- `read_chan_signer` is legacy fallback only (pre-v0.0.113 channels).

**Acceptance**:
- [x] `cargo check` passes (0 errors from HybridSigner code)
- [x] `HybridSigner` implements `ChannelSigner` + `EcdsaChannelSigner` + `Clone` + `Writeable`
- [x] `TeeSignerFactory` injectable via `NodeBuilder`
- [ ] Phase 2: Replace `Tee(InMemorySigner)` with `Tee(Atecc608aSigner)` — requires hardware
- [ ] Phase 2: Wrap I2C calls in `spawn_blocking` to prevent LDK thread starvation
</details>

<details>
<summary><strong>Step 4.2: Latency Benchmarking</strong> (2-3 days)</summary>

**File**: `benchmarks/latency_benchmark.rs`

**Measures**:
- ATECC608A signing: 100 samples, calculate avg/p99
- Hot key signing: 1000 samples, verify < 5ms
- I2C bus contention: concurrent operations

**Acceptance**:
- [ ] ATECC608A: avg < 50ms, p99 < 100ms
- [ ] Hot keys: avg < 5ms
- [ ] No I2C bus errors
</details>

<details>
<summary><strong>Step 4.3: Lightning Network Testing</strong> (3-4 days)</summary>

**Test sequence**:
```bash
# Alice (hardware) + Bob (mock)
# 1. Open channel (1M sats)
# 2. Forward HTLC (10k sats, triggers hot path)
# 3. Close channel (triggers cold path)
```

**Acceptance**:
- [ ] Channel opens (commitment signed by ATECC608A)
- [ ] HTLC forwards without timeout
- [ ] Channel closes successfully
- [ ] No Lightning errors
</details>

### Acceptance Criteria (Milestone 4)

- [ ] LDK integration complete with custom signer
- [ ] HTLC < 500k sats use hot path (< 5ms latency)
- [ ] Commitment txs use ATECC608A (< 100ms latency)
- [ ] Channel operations (open/forward/close) successful
- [ ] No Lightning HTLC timeouts under normal load
- [ ] Performance benchmarks documented

### Dependencies

- LDK node running (existing infra)
- Bitcoin regtest environment (existing)
- Channel testing scripts

---

---

## Post-Milestone 4: Production Hardening

**Timeline**: Week 8+ (ongoing)
**Goal**: Security enhancements for production deployments

### Future Enhancements

| Enhancement | Timeline | Benefit |
|-------------|----------|---------|
| **Encrypted I2C** | Week 9-10 | Mitigates bus sniffing attacks |
| **Transparency Log** | Week 10-12 | Community-driven firmware verification |
| **Hardware Packaging** | Week 12+ | Tamper-evident casing, epoxy coating |
| **Certification** | 6-12 months | FIPS 140-3 Level 2, Common Criteria EAL4+ |

**Note**: These are optional enhancements for highest-assurance deployments.

---

## Risk Mitigation

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Hardware supply chain delays** | Medium | High | Order hardware early, use breadboard prototypes for Milestone 3 |
| **I2C latency higher than expected** | Low | High | Benchmark early (Milestone 3.3), implement async signing queue if needed |
| **LDK API incompatibility** | Low | Medium | Review LDK SignerProvider trait early, engage with LDK team if needed |
| **Debian RO root breaks systemd** | Medium | Medium | Test thoroughly on Radxa hardware (Milestone 2.4), upstream fixes if needed |
| ~~**ESP32 firmware bugs**~~ | ⛔ | DROPPED | ESP32-C6 gatekeeper removed from architecture |

---

## Implementation Starting Point

### When you're ready to start coding

**First action**: Create module structure (5 minutes)

```bash
cd packages/backend/src
mkdir -p tee/{providers,tests}
touch tee/{mod.rs,error.rs,config.rs,traits.rs}
touch tee/providers/{mod.rs,mock_hardware.rs,mock_vls.rs}
touch tee/tests/attestation_test.rs
```

**First file to code**: [`packages/backend/src/tee/mod.rs`](../../packages/backend/src/tee/mod.rs)

```rust
//! Trusted Execution Environment (TEE) module
pub mod config;
pub mod error;
pub mod traits;
pub mod providers;

pub use config::TeeConfig;
pub use error::TeeError;
pub use traits::{HardwareTrust, VlsProvider};
```

**Then follow**: Milestone 1 → Step 1.0 → 1.1 → 1.2 → ... (see above)

**Estimated completion**:
- Milestone 1 (Week 1-2): 10-15 hours
- Milestone 2 (Week 2-3): 10-13 hours
- Milestone 3 (Week 4-6): 3-4 weeks
- Milestone 4 (Week 6-8): 2-3 weeks

**Total for core functionality**: 6-8 weeks

---

**See Also**:
- [README.md](README.md) - Feature overview and quick start
- [ARCHITECTURE.md](ARCHITECTURE.md) - Detailed system design
- [SECURITY_MODEL.md](SECURITY_MODEL.md) - Threat model and mitigations
- [plan.md](plan.md) - Implementation plan with constitution check
