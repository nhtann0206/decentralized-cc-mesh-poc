# Confidential Compute Architecture

## System Overview

Confidential Compute adds hardware-backed security to Lightning Node OS through **physical chip isolation** and **measured boot**. The design separates security-critical operations into dedicated hardware modules while maintaining the existing five-layer architecture.

## Architecture Principles

1. **Additive Only**: Zero modifications to existing services/endpoints/repositories
2. **Trait Abstraction**: Hardware providers implement standard traits (same pattern as Repository)
3. **Feature-Gated**: `TEE_ENABLED=false` by default - production code switches via env var
4. **Mock-First**: Development without hardware dependencies
5. **Sub-Project Separation**: U-Boot/TrustZone TA firmware isolated from Rust application (~~ESP32 DROPPED~~)

## Hardware Architecture

### Component Topology

> **Architecture Change (2026-02-25)**: ESP32-C6 officially **DROPPED**. ATECC608A retained as Root of Trust. Radxa communicates directly with ATECC608A and TPM via I2C bus. Boot verification handled by kernel-level TPM check or ARM TrustZone OP-TEE.

```
┌──────────────────────────────────────────────────────────────────────┐
│              HARDWARE TOPOLOGY (POST-ESP32 DROP)                     │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │   Radxa Zero 3W (Application CPU — RK3566 ARM64)            │   │
│  │                                                              │   │
│  │  ┌────────────────────────┐  ┌────────────────────────────┐ │   │
│  │  │ Linux (Debian Minimal) │  │ VLS Provider               │ │   │
│  │  │  - Read-Only RootFS    │  │ (HybridSigner via LDK fork)│ │   │
│  │  │  - zRAM Overlay        │  │  - Hot: RAM keys (<5ms)    │ │   │
│  │  │  - systemd WatchdogSec │  │  - Cold: ATECC keys(<100ms)│ │   │
│  │  │  - Node Backend (Rust) │  │  - TeeSignerFactory inject │ │   │
│  │  └────────────────────────┘  └─────────────┬──────────────┘ │   │
│  │                                             │ I2C            │   │
│  └─────────────────────────────────────────────┼────────────────┘   │
│                                                │                     │
│                          I2C Bus (400kHz) ─────┘                     │
│                                │                                     │
│  ┌─────────────────────────────┴─────────────────────────────────┐  │
│  │  Security Hat (I2C peripherals)                                │  │
│  │  ┌──────────────┐          ┌──────────────┐                   │  │
│  │  │ ATECC608A    │          │ TPM 2.0      │                   │  │
│  │  │ (0x60)       │          │ (0x2E)       │                   │  │
│  │  │ - Slot 0-15  │          │ - PCR 0-23   │                   │  │
│  │  │ - ECC P-256  │          │ - NVRAM      │                   │  │
│  │  │ - Key never  │          │ - Boot meas. │                   │  │
│  │  │   exported   │          │ - Tamper-    │                   │  │
│  │  └──────────────┘          │   evident    │                   │  │
│  │                            └──────────────┘                   │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

> **Note**: ARM TrustZone OP-TEE may provide additional on-die secure execution for boot verification. The ATECC608A remains the Root of Trust for key material isolation regardless. See `docs/conf-com/research/tee/architecture_synthesis_phase3.md`.

### Communication Protocols

> ~~GPIO RESET and UART~~ **DROPPED** — ESP32 removed. Only I2C remains.

| Interface | Speed | Purpose | Protocol |
|-----------|-------|---------|----------|
| **I2C (ATECC608A)** | 400 kHz Fast Mode | Signing operations | I2C read/write commands via Actor pattern |
| **I2C (TPM 2.0)** | 400 kHz Fast Mode | Boot measurements | TPM2 protocol (PCR read/extend) |

### Pin Mapping

**Radxa Zero 3W Connections (direct I2C to Security Hat)**:
- Pin 3 (I2C SDA): Data line to ATECC608A (0x60) + TPM 2.0 (0x2E)
- Pin 5 (I2C SCL): Clock line to ATECC608A + TPM 2.0

## Software Architecture

### Trait Abstraction Layer

```rust
// File: packages/backend/src/tee/traits.rs
// NOTE: Synced with actual implementation (Phase 1, commit 5474ff9f)

/// Boot integrity verification (TPM PCR-10 + ATECC608A attestation). ~~ESP32 DROPPED~~
#[async_trait]
pub trait HardwareTrust: Send + Sync {
    /// Get attestation from hardware (TPM + ATECC608A via I2C). ~~ESP32 gatekeeper DROPPED~~
    async fn get_boot_attestation(&self) -> Result<BootAttestation, TeeError>;

    /// Read specific TPM PCR value (PCR-10: Kernel + DTB + Initramfs)
    async fn read_tpm_pcr(&self, pcr: u8) -> Result<[u8; 32], TeeError>;

    /// Send heartbeat to systemd watchdog (WatchdogSec=60, call every <30s). ~~ESP32 watchdog DROPPED~~
    async fn send_heartbeat(&self) -> Result<(), TeeError>;

    /// Respond to a remote attestation challenge with signed measurements
    async fn attest(&self, challenge: AttestationChallenge) -> Result<AttestationResponse, TeeError>;
}

pub struct BootAttestation {
    pub nonce: [u8; 32],           // Hardware RNG from ATECC608A (~~ESP32 DROPPED~~)
    pub kernel_hash: [u8; 32],     // TPM PCR-10 value
    pub signature: Vec<u8>,        // ECDSA sig (64 bytes). Vec<u8> not [u8;64] due to serde limit
    pub verified_at: SystemTime,
    pub is_hardware_backed: bool,  // true = hardware (ATECC608A+TPM), false = mock
}

pub struct AttestationChallenge {
    pub requester_node_id: String,
    pub challenge_nonce: [u8; 32],
    pub timestamp: SystemTime,
}

pub struct AttestationResponse {
    pub boot_measurement: BootAttestation,
    pub platform_info: PlatformInfo,    // PCR values included inside PlatformInfo
}

pub struct PlatformInfo {
    pub platform: String,                      // "radxa-zero-3w", "mock", "qemu"
    pub firmware_version: String,              // Platform/OS version (~~ESP32 gatekeeper DROPPED~~)
    pub tpm_pcr_values: Vec<(u8, [u8; 32])>,  // Populated PCR registers
}
```

### VLS Provider (Hybrid Signing)

```rust
// File: packages/backend/src/tee/traits.rs
// NOTE: Synced with actual implementation (Phase 1, commit 5474ff9f)

/// Lightning signing operations with hardware isolation
#[async_trait]
pub trait VlsProvider: Send + Sync {
    /// Sign arbitrary data (returns DER-encoded ECDSA signature)
    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, TeeError>;

    /// Sign a commitment transaction (always cold path via ATECC608A)
    async fn sign_commitment(&self, tx_hash: &[u8; 32]) -> Result<Vec<u8>, TeeError>;

    /// Sign an HTLC (hot path if < threshold, cold path otherwise)
    /// Returns signature + which strategy was used
    async fn sign_htlc(
        &self,
        htlc_hash: &[u8; 32],
        amount_msat: u64,
    ) -> Result<(Vec<u8>, SigningStrategy), TeeError>;

    /// Returns whether this provider is backed by real hardware
    fn is_hardware_backed(&self) -> bool;
}

/// Signing strategy selection for hybrid hot/cold path.
///
/// This is a simple enum indicating which path was taken — it does NOT
/// contain key material or internal state. Key management is an internal
/// detail of each VlsProvider implementation, never exposed via public API.
pub enum SigningStrategy {
    /// Hot keys in RAM (zeroize-protected). Latency: ~1ms.
    /// Used for: HTLC forwarding < 500k sats
    HotPath,

    /// Cold keys in ATECC608A (hardware isolated). Latency: 25-100ms.
    /// Used for: Commitments, channel closes, large HTLCs
    ColdPath,
}
```

> **Design note**: `sign_closing` (channel close signing) is a Milestone 4 candidate
> when integrating with LDK `SignerProvider`. Currently, `sign_commitment` covers
> commitment transactions and `sign_htlc` covers forwarding. Channel close can
> reuse `sign_commitment` since both are always-cold-path operations.

```rust
// Example: How the provider selects strategy internally (mock_vls.rs)
const HTLC_HOT_PATH_THRESHOLD_MSAT: u64 = 500_000_000; // 500k sats

async fn sign_htlc(&self, htlc_hash: &[u8; 32], amount_msat: u64)
    -> Result<(Vec<u8>, SigningStrategy), TeeError>
{
    let strategy = if amount_msat < HTLC_HOT_PATH_THRESHOLD_MSAT {
        SigningStrategy::HotPath
    } else {
        SigningStrategy::ColdPath
    };
    // ... sign and return (signature, strategy)
}
```

### Integration with Existing Architecture

**No changes to five-layer architecture**. TEE integrates via dependency injection:

```rust
// File: packages/backend/src/state.rs (field definitions)
// File: packages/backend/src/bootstrap/phase_app_state.rs (initialization)
// NOTE: Synced with actual implementation (Phase 1, commit ff164535)

// --- state.rs ---
pub struct AppState {
    // ... existing 50+ fields ...

    /// TEE hardware trust provider for boot attestation and watchdog
    /// None when TEE_ENABLED=false (default)
    pub tee_trust: Option<Arc<dyn HardwareTrust>>,

    /// TEE VLS provider for hybrid Lightning signing
    /// None when TEE_ENABLED=false (default)
    pub tee_vls: Option<Arc<dyn VlsProvider>>,
}

// --- bootstrap/phase_app_state.rs (Phase 5 of 8-phase bootstrap) ---
pub fn execute(/* foundation, database, infra, services */) -> Result<Arc<AppState>, AppError> {
    // Initialize TEE providers (039-confidential-compute)
    let tee_config = crate::tee::TeeConfig::from_env();
    let (tee_trust, tee_vls) = crate::tee::initialize_providers(&tee_config)
        .map_err(|e| AppError::Configuration(format!("TEE initialization failed: {}", e)))?;

    let app_state = Arc::new(AppState {
        // ... existing fields ...
        tee_trust,
        tee_vls,
    });
    Ok(app_state)
}

// --- tee/mod.rs (factory function) ---
pub fn initialize_providers(config: &TeeConfig)
    -> Result<(Option<Arc<dyn HardwareTrust>>, Option<Arc<dyn VlsProvider>>), TeeError>
{
    if !config.enabled { return Ok((None, None)); }  // Zero overhead when disabled
    match &config.provider {
        TeeProviderType::Mock => { /* MockHardwareTrust + MockVlsProvider */ }
        TeeProviderType::Hardware { .. } => { /* Milestone 3: RadxaHardware (I2C) + Atecc providers. ~~Esp32 DROPPED~~ */ }
    }
}
```

## 3-Tier Core Injection Strategy

Confidential Compute secures the "Economic Internet" of the Node by applying protection at three critical injection points (The Vault, The Gateway, and The Sandbox), balancing strict hardware isolation with the high-performance demands of API routing.

### Tier 1: The Vault (LDK Funds & Identity Protection)
*   **Target**: `ldk-node` wallet and channel keys.
*   **Vulnerability**: LDK stores the `signer_seed.hex` as plaintext on disk and loads commitment/HTLC keys directly into RAM.
*   **Injection Plan (M4 — Implemented)**:
    1.  **HybridSigner Enum**: `WalletKeysManager::EcdsaSigner` changed from `InMemorySigner` to `HybridSigner` enum with `Software` and `Tee` variants. Runtime dispatch via `TeeSignerFactory` trait injected through `NodeBuilder::set_tee_signer_factory()`.
    2.  **Type System Validated**: `ChainMonitor<HybridSigner>` compiles. 12 `Arc<KeysManager>` references auto-resolve. Zero backward compat risk (LDK doesn't serialize signers since v0.0.113).
    3.  **Seed Encryption at Rest**: Use an ATECC608A-derived wrapping key to encrypt `signer_seed.hex` before writing to SQLite/disk. Even if the disk is stolen, the seed cannot be decrypted without the physical ATECC chip.
*   **Known Limitations**:
    *   ATECC608A is a **blind signer** — signs any hash without channel state validation. Does NOT prevent unauthorized signing under RCE.
    *   **BDK On-chain Wallet** shares the same seed and remains a Hot Wallet in RAM.
    *   **NodeSigner** (node identity) still delegates to RAM `KeysManager`.

### Tier 2: The Gateway (L402 API Monetization)
*   **Target**: `L402Service` macaroon root key.
*   **Vulnerability**: The L402 `macaroon_root_key` is stored in SQLite (plaintext). Memory dumps or SQL injection allows attackers to forge unlimited Macaroons for free API/bandwidth usage.
*   **Injection Plan**:
    1.  **ATECC608A Wrapping Key**: Generate a static wrapping key from ATECC608A (Slot 1) upon boot.
    2.  **Key Persistence & Isolation**: The `macaroon_root_key` is encrypted with AES-256-GCM using the wrapping key before saving to SQLite.
    3.  **High-Performance Validation**: Upon boot, the Database decrypts the Macaroon Root Key using the ATECC608A wrapping key and loads the plaintext root key into `SecureMemory` (RAM, `mlock`, zeroize). The L402 Middleware validates incoming requests against RAM at maximum speed, avoiding the 100ms I2C penalty. Crash restarts maintain Macaroon continuity.

### Tier 3: The Sandbox (AI Agent Execution Isolation)
*   **Target**: `AgentExecutor` and backend memory space.
*   **Vulnerability**: AI Agents execute untrusted inputs (LLM prompt injections) in the same process space as the Lightning Node.
*   **Injection Plan**:
    1.  **Seal Semantic Access (Vector A - `Sprint 039 Quick Fix`)**: Remove direct `Arc<ldk_node::Node>` references from `AgentExecutor` and the Agent Tools pipeline. This ensures a Prompt Injection cannot trick the LLM into invoking LDK methods via the tool interface.
    2.  **Hardware Fallback (Vector B - `The Vault`)**: If an attacker achieves True RCE (memory corruption) inside the Rust process, the hardware isolation of the ATECC608A ensures channel keys cannot be **extracted** from RAM. However, ATECC608A is a blind signer and CANNOT prevent the attacker from using the keys to sign malicious transactions. Full protection requires VLS (Validating Signer) or M7 WASM Sandbox.
    3.  **Full Process/WASM Isolation (`Future Sprint - M7`)**: Ultimately decouple AI execution from the primary Node process into WASM sandboxes to physically prevent RCE-driven memory access.

## Measured Boot Flow

### Pre-Bootstrap Sequence (Before 8-phase backend bootstrap)

> **Architecture Change (2026-02-25)**: ESP32 challenge-response boot flow **DROPPED**. Replaced by kernel-level TPM verification or ARM TrustZone OP-TEE.

```
┌─────────────────────────────────────────────────────────────────┐
│              MEASURED BOOT SEQUENCE (POST-ESP32 DROP)            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  [1] Power On                                                    │
│      └─ Radxa Zero 3W boots U-Boot normally                     │
│                                                                  │
│  [2] U-Boot Measurement                                          │
│      ├─ U-Boot hashes: Kernel + DTB + Initramfs → SHA256        │
│      └─ U-Boot extends TPM PCR-10 with hash via I2C             │
│                                                                  │
│  [3] Kernel Boot                                                 │
│      ├─ Linux kernel boots from read-only root partition         │
│      └─ Kernel reads TPM PCR-10 to verify boot integrity        │
│                                                                  │
│  [4] Backend Startup                                             │
│      ├─ Node Backend reads TEE_ENABLED, TEE_PROVIDER env vars   │
│      ├─ If TEE_ENABLED=true: read TPM PCR-10 via I2C            │
│      │   Compare against known-good hash (TEE_KERNEL_HASH)      │
│      │   If mismatch → log critical error, enter degraded mode  │
│      ├─ Initialize ATECC608A via I2C for signing operations     │
│      └─ Start systemd watchdog (WatchdogSec=60)                 │
│                                                                  │
│  [5] Remote Attestation (on request)                             │
│      ├─ Peer sends challenge with nonce                          │
│      ├─ Backend reads TPM PCR values + signs with ATECC608A     │
│      └─ Returns attestation response to peer                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

> **Future**: ARM TrustZone OP-TEE can provide Secure World boot verification before Normal World Linux starts, adding a stronger trust anchor.

### Boot Integrity Guarantee

**Chain of Trust**:
1. **ATECC608A Private Key**: Provisioned once, locked forever (Root of Trust)
2. **TPM PCR-10**: Extends with kernel hash, tamper-evident
3. **U-Boot Measurement**: Hashes kernel before boot
4. **Backend Verification**: Reads TPM PCR-10 and compares against known-good value

**Attack Mitigation**:
- Modified kernel → Different hash → TPM PCR-10 mismatch → Backend detects tampering
- Remote attestation → Peer nodes verify boot measurements
- Read-only root → Kernel on immutable partition, modification requires physical SD card swap

## Debian Hardening

### Read-Only Root Filesystem

```bash
# File: infra/debian-minimal/setup_readonly_rootfs.sh

#!/bin/bash
# Configure Debian for read-only root filesystem

# /etc/fstab modifications
cat >> /etc/fstab <<EOF
# Read-only root
/dev/mmcblk0p2  /               ext4    ro,noatime              0 1

# Writable overlay (zRAM-backed)
tmpfs           /run            tmpfs   defaults,size=64M       0 0
tmpfs           /var/log        tmpfs   defaults,size=32M       0 0
tmpfs           /tmp            tmpfs   defaults,size=128M      0 0

# Persistent Lightning data
/dev/mmcblk0p3  /var/lib/ldk    ext4    defaults,noatime        0 2
EOF

# Remount root as read-only
mount -o remount,ro /
```

### zRAM Overlay Configuration

```bash
# File: infra/debian-minimal/setup_zram_overlay.sh

#!/bin/bash
# Setup zRAM-backed writable overlay

# Load zRAM module
modprobe zram num_devices=1

# Configure 256MB zRAM device
echo 268435456 > /sys/block/zram0/disksize  # 256MB
mkfs.ext4 /dev/zram0

# Mount overlay
mkdir -p /run/overlay-{upper,work}
mount -t overlay overlay \
    -o lowerdir=/,upperdir=/run/overlay-upper,workdir=/run/overlay-work \
    /overlay

# Bind mount writable paths
mount --bind /overlay/etc /etc
mount --bind /overlay/var /var
```

### Persistent Storage Strategy

| Path | Storage | Rationale |
|------|---------|-----------|
| `/` (root) | Read-only SD card | Immutable system binaries |
| `/var/lib/ldk` | Persistent partition | Lightning channel state (CRITICAL) |
| `/etc` | zRAM overlay | Temp config changes (lost on reboot) |
| `/var/log` | tmpfs (RAM) | Logs expire, queried via Activity API |
| `/tmp` | tmpfs (RAM) | Temp files (standard practice) |

## Directory Structure

```
packages/backend/src/tee/                          # ✅ = implemented, 📋 = Milestone 3+
├── mod.rs                          # ✅ Module exports + initialize_providers()
├── error.rs                        # ✅ TeeError enum (8 variants)
├── config.rs                       # ✅ TeeConfig + TeeProviderType + from_env()
├── traits.rs                       # ✅ HardwareTrust + VlsProvider traits + data structs
├── cobs.rs                         # ⛔ TO DROP — ESP32 UART removed (C-006 DROPPED)
├── i2c_actor.rs                    # ✅ I2C Actor pattern: mpsc + oneshot + MockHandler (C-007)
├── watchdog.rs                     # ✅ Two-layer watchdog: HealthAggregator + sd_notify (FR-012). ~~ESP32 layer DROPPED~~
├── health.rs                       # ✅ HealthAggregator + HealthProbe trait (T6 mitigation)
├── probes.rs                       # ✅ DatabaseHealthProbe + LdkNodeHealthProbe
├── secure_memory.rs                # ✅ SecureMemory<T>: mlock + zeroize (T2 mitigation)
├── attestation_registry.rs         # ✅ AttestationRegistry trait + mock (T8 mitigation)
├── ldk_signer_adapter.rs           # ✅ VlsSignerProvider: LDK bridge (Milestone 4 prep)
├── providers/
│   ├── mod.rs                      # ✅ Re-exports (mock + hardware stubs)
│   ├── mock_hardware.rs            # ✅ MockHardwareTrust (dev/CI)
│   ├── mock_vls.rs                 # ✅ MockVlsProvider (software signing + SecureMemory)
│   ├── esp32_hardware.rs           # ⛔ TO DROP — ESP32 removed from architecture
│   ├── tpm_hardware.rs             # 📋 TPM 2.0 I2C driver [Milestone 3]
│   └── atecc_vls.rs                # ✅ AteccVlsProvider stub (I2C signing) [needs hardware]

packages/backend/src/transport/node_server/
└── http_tee.rs                     # ✅ POST /tee/attest, GET /tee/status

packages/backend/tests/
└── tee_attestation_test.rs         # ✅ 5 integration tests (Alice-Bob, strategy, etc.)

esp32/src/                              # ⛔ ENTIRE DIRECTORY DROPPED — ESP32 removed

infra/qemu-tee/                         # ✅ QEMU ARM64 environment + Debian hardening
├── create_disk_images.sh               # Download Debian cloud image + persistent disk
├── launch_qemu.sh                      # QEMU ARM64 with HVF, port forwarding
├── setup_readonly_rootfs.sh            # Read-only root configuration
├── setup_zram_overlay.sh               # zRAM overlay setup
├── provision_persistent.sh             # /var/lib/ldk persistent partition
├── validate_all.sh                     # 6-category validation suite
├── cleanup.sh                          # Tear down QEMU
├── tee-watchdog.service                # Type=notify, WatchdogSec=60
└── var-lib-ldk.mount                   # /var/lib/ldk mount unit
```

## Performance Characteristics

### Latency Breakdown

| Operation | Hot Path (RAM) | Cold Path (ATECC608A) | Requirement |
|-----------|----------------|----------------------|-------------|
| **HTLC forward** | 0.5-1ms | 25-100ms | < 5ms (hot path used) |
| **Commitment sign** | N/A | 25-100ms | < 200ms (acceptable) |
| **Channel close** | N/A | 25-100ms | < 200ms (acceptable) |
| **Boot attestation** | N/A | 150-300ms | < 1s (one-time) |
| **Watchdog heartbeat** | N/A | 5-10ms | < 30s (ample margin) |

### I2C Bus Contention

**Shared Bus**: ATECC608A (0x60) + TPM 2.0 (0x2E) on same I2C-1

**Mitigation** (see constraint C-001, C-007):

> **CRITICAL**: MUST use `tokio::sync::Mutex`, NEVER `std::sync::Mutex`.
> Using `std::sync::Mutex` in async context blocks the tokio worker thread,
> causing server-wide stalls when ATECC608A takes 25-100ms to sign.

```rust
// File: packages/backend/src/tee/providers/mod.rs (Milestone 3 design)
//
// Preferred: Actor pattern via mpsc channel (see constraint C-007).
// Fallback: tokio::sync::Mutex with timeout.

use tokio::sync::Mutex;  // NOT std::sync::Mutex
use tokio::time::{timeout, Duration};

lazy_static! {
    static ref I2C_BUS: Mutex<()> = Mutex::new(());
}

impl AteccVlsProvider {
    async fn atecc_sign(&self, slot: u8, data: &[u8]) -> Result<Vec<u8>, TeeError> {
        // Timeout prevents indefinite blocking if I2C bus is stuck
        let _lock = timeout(Duration::from_millis(200), I2C_BUS.lock())
            .await
            .map_err(|_| TeeError::I2cError("I2C bus lock timeout (200ms)".into()))?;

        let start = Instant::now();
        let signature = self.i2c_device.sign(slot, data).await?;
        let latency = start.elapsed();

        if latency > Duration::from_millis(10) {
            warn!(target: "node_backend::tee",
                  latency_ms = latency.as_millis(),
                  "I2C lock contention detected");
        }

        Ok(signature)
    }
}
```

## Security Considerations

### Threat Model

See [SECURITY_MODEL.md](SECURITY_MODEL.md) for comprehensive threat analysis.

### Key Security Properties

1. **Boot Integrity**: Kernel tampering detected before Linux starts
2. **Key Isolation**: Private keys physically isolated in ATECC608A
3. **Stateless Security**: Read-only root prevents persistent malware
4. **Watchdog Enforcement**: System killed if backend hangs/crashes
5. **Remote Attestation**: Other nodes can verify boot measurements

### Security Trade-offs

| Feature | Security Gain | Usability Cost |
|---------|---------------|----------------|
| **Read-only root** | Persistent malware impossible | Config changes require reboot |
| **Hardware keys** | Key extraction impossible | I2C latency (25-100ms) |
| **Watchdog (60s)** | Crash detection | Must heartbeat every 30s |
| **Boot verification** | Tamper detection | 1-2s boot delay |

---

**See Also**:
- [SECURITY_MODEL.md](SECURITY_MODEL.md) - Threat model and attack mitigations
- [ROADMAP.md](ROADMAP.md) - Milestone-by-milestone implementation plan
