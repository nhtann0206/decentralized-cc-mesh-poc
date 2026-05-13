# Security Model and Threat Analysis

## Overview

This document analyzes the threat landscape for confidential compute and how hardware isolation mitigates specific attack vectors. The security model assumes **local physical access** attacks are out of scope (requires physical tamper-evident packaging for production hardware).

## Threat Model Scope

### In Scope

✅ **Software Attacks**:
- Malicious code injection via compromised dependencies
- Memory dumps to extract private keys
- Kernel rootkits and bootkit attacks
- Persistent malware on filesystem
- Network-based exploits targeting backend services

✅ **Operational Attacks**:
- Admin credential compromise
- SSH access with malicious intent
- Insider threats (physical access to device)
- Supply chain attacks (compromised firmware updates)

### Out of Scope

❌ **Physical Attacks Requiring Specialized Equipment**:
- Hardware debugging (JTAG, SWD) to extract chip secrets
- Fault injection (voltage/clock glitching)
- Side-channel analysis (power/EM/timing)
- Chip decapping and microprobing

**Rationale**: Physical attacks require lab equipment, expertise, and physical access. Mitigation would require tamper-evident packaging, epoxy coating, and physical security (standard HSM practices). This is **future work** for production deployments requiring highest assurance.

## 3-Phase OP-TEE Roadmap

The security architecture follows a 3-phase evolution aligned with hardware availability.
The threat model from "physical disassembly reveals nothing" is only fully realized in Phase 2.

| Phase | Hardware | Key Deliverable | Trigger |
|-------|----------|-----------------|---------|
| **Phase 1 (Now)** | MacOS + QEMU | Software hardening + OP-TEE-ready interfaces | Done |
| **Phase 2** | Radxa Zero 3W | OP-TEE on RK3566 + Key Manager TA + TrustZone isolation | Radxa delivery + R-012 |
| **Phase 3** | Security HAT | ATECC608A attestation + VLS in Secure World | HAT delivery |

### Phase 1 — Software Hardening (Complete)

All key material protected by software-layer encryption inside LUKS2 container:

- `signer_seed.enc` — AES-256-GCM with HKDF-SHA256 wrap key (T-403) ✅
- `macaroon_root_key` — AES-256-GCM encrypted in SQLite with `enc:` prefix (T-501) ✅
- `.wrap_key` — file-backed, mode 0600, inside LUKS2 container (T-801) ✅
- Seed buffers — `Zeroizing<[u8;64]>` prevents RAM remnants (P1-1) ✅

**Residual Phase 1 risk**: An attacker who gains both disk access AND root on a running
system can read the `.wrap_key` file and decrypt all secrets. Phase 2 eliminates this.

### Phase 2 — OP-TEE TrustZone (Radxa + R-012)

Wrap key generated in OP-TEE Secure World, sealed to device HUK + PCR state:
- Normal World NEVER receives the raw wrap key — only a derived AES key through `/dev/tee0`
- LUKS2 passphrase sealed to PCR0+PCR7+PCR10 — tampered boot = locked disk
- secp256k1 Lightning signing inside Trusted Application (resolves R-003)

### Phase 3 — ATECC608A Attestation (Security HAT)

ATECC608A complements OP-TEE with independent physical attestation:
- Physical tamper detection (independent of OS)
- VLS validation logic in Trusted Application (resolves R-001 Blind Signer Paradox)

---

## Asset Classification

| Asset | Criticality | Phase 1 Protection | Phase 2 Protection |
|-------|-------------|--------------------|--------------------|
| **Lightning Channel State** | CRITICAL | LUKS2 ext4 partition | Same + PCR-sealed unlock |
| **Lightning Signing Key (seed)** | CRITICAL | AES-256-GCM, key in LUKS2 `.wrap_key` | Key in OP-TEE Secure Storage, sealed to HUK+PCR |
| **On-chain Wallet Keys (BDK)** | CRITICAL | Same as seed (shared key) | Same as seed Phase 2 |
| **Node Identity Key** | HIGH | RAM (`KeysManager.inner`) — NOT PROTECTED | OP-TEE Secure World (Phase 3 scope) |
| **Ephemeral Hot Keys** | HIGH | `Zeroizing<>` in RAM, cleared on drop | Same |
| **L402 Macaroon Root Key** | HIGH | AES-256-GCM in SQLite, key in LUKS2 `.wrap_key` | Key in OP-TEE Secure Storage |
| **Boot Measurements** | HIGH | TPM PCR-10 measured, not policy-gated | PCR policy gates LUKS2 unlock |
| **JWT Tokens** | MEDIUM | Client-side, short-lived (15min) | Same |
| **Disk Contents** | HIGH | LUKS2 AES-256-GCM full-disk encryption | Same + passphrase PCR-sealed |

## Threat Scenarios and Mitigations

### T1: Malicious Kernel Module (Bootkit)

**Attack Vector**:
Attacker modifies kernel or bootloader to load rootkit during boot. Rootkit intercepts signing operations and exfiltrates private keys.

**Mitigations**:
1. ✅ **Measured Boot**: U-Boot extends TPM PCR-10 with kernel hash. Backend verifies PCR-10 against known-good value on startup. ~~ESP32 gatekeeper~~ DROPPED.
2. ✅ **Signature Verification**: U-Boot measures kernel hash, stored in TPM PCR-10 (tamper-evident)
3. ✅ **Read-Only Root**: Kernel stored on read-only partition, modification requires physical SD card swap
4. ✅ **Boot Integrity Check**: Backend compares TPM PCR-10 against `TEE_KERNEL_HASH` env var. Mismatch → degraded mode + critical log. ~~ESP32 holds RESET low~~ DROPPED.

**Residual Risk**: LOW
- Attacker would need to:
  1. Physical access to swap SD card
  2. ATECC608A private key to generate valid signature
  3. Both are infeasible without physical security compromise

**Detection**:
- Remote attestation: Other nodes request boot measurements, detect mismatch
- Monitoring: Failed boot attempts logged to external syslog server

---

### T2: Memory Dump Attack (Extract Private Keys)

**Attack Vector**:
Attacker gains root access (via SSH or exploit) and dumps `/dev/mem` to extract Lightning private keys from RAM.

**Mitigations**:
1. ✅ **Hardware Key Isolation**: Lightning channel signing keys derived inside ATECC608A via `HybridSigner::Tee` path, never exported to RAM
2. ✅ **Zeroize Ephemeral Keys**: Hot keys (HTLCs) use `zeroize` crate, memory cleared on drop
3. ✅ **Limited Exposure**: Hot keys only used for low-value HTLCs (below configurable threshold, default 500k sats)
4. ⚠️ **Process Isolation**: Node backend runs as non-root user, cannot dump kernel memory directly

**Known Gaps (T2)**:
- **BDK On-chain Wallet remains Hot**: LDK-Node uses the same 64-byte seed for both `KeysManager` (Lightning) and `BdkWallet` (on-chain). The `xprv` root key MUST be in RAM for BDK on-chain operations (deposit/withdraw). Fix requires BDK hardware signer (PSBT signing via hardware) — Phase 3 scope.
- **NodeSigner keys in RAM**: `WalletKeysManager::NodeSigner` impl delegates to `self.inner` (RAM `KeysManager`). `node_secret_key` remains exposed. Fix requires OP-TEE Trusted Application (Phase 2/3).

**Mitigated (Phase 1)**:
- ✅ Seed encrypted at rest via AES-256-GCM + HKDF-SHA256 (T-403) — disk extraction yields ciphertext only
- ✅ Seed buffers use `Zeroizing<[u8; 64]>` — memory cleared on drop, no swap remnants
- ✅ LUKS2 full-disk encryption (T-801) — all files in `/var/lib/ldk/` encrypted at block level

**Residual Risk**: MEDIUM (Phase 1) → LOW (Phase 2, after OP-TEE)
- Phase 1: Secrets protected from disk extraction; RAM dump by root still possible
- Phase 2: Wrap key in OP-TEE Secure World — even root cannot access raw key material
- **Mitigation (Phase 1)**: LUKS2 + AES-GCM at-rest encryption; `Zeroizing<>` for RAM safety

**Detection**:
- SELinux/AppArmor policies: Block `/dev/mem` access
- Audit logs: Monitor memory access patterns

---

### T3: Persistent Malware (Filesystem Compromise)

**Attack Vector**:
Attacker installs persistent backdoor in `/usr/bin/node-backend` or systemd service files. Backdoor activates on reboot and intercepts signing requests.

**Mitigations**:
1. ✅ **Read-Only Root**: All binaries on read-only partition
2. ✅ **zRAM Overlay**: Write attempts go to RAM, lost on reboot
3. ✅ **No Persistent /etc**: Config changes ephemeral unless explicitly persisted
4. ✅ **Measured Boot**: Modified binaries change kernel hash → boot fails

**Residual Risk**: LOW
- Attacker cannot persist changes without physical SD card swap
- zRAM overlay provides forensic visibility (changes logged but not persisted)

**Detection**:
- File integrity monitoring (AIDE/Tripwire): Detect attempts to modify read-only files
- Boot measurement comparison: Hash mismatch indicates tampering

---

### T4: Supply Chain Attack (Compromised Firmware Update)

**Attack Vector**:
Attacker compromises OTA update server and pushes malicious firmware update. Update includes backdoored kernel or node backend binary.

**Mitigations**:
1. ✅ **Signed Updates**: All OTA updates signed with developer private key
2. ✅ **Signature Verification**: Update service verifies signature before applying
3. ✅ **Rollback Protection**: TPM NVRAM stores update counter, prevents downgrade attacks
4. ⚠️ **Transparency Log**: All updates logged to public transparency log (planned)

**Residual Risk**: MEDIUM (without transparency log)
- Attacker needs developer private key to sign malicious update
- Without transparency log, compromised key can push backdoor undetected

**Detection**:
- Update signature verification (enforced)
- Transparency log monitoring (planned - Milestone 7)
- Community-driven checksum verification

---

### T5: Lightning Commitment Transaction Manipulation

**Attack Vector**:
Attacker gains access to node backend and modifies commitment transaction signing logic to broadcast old states (attempt to steal funds).

**Mitigations**:
1. ✅ **Hardware-Backed Signing**: Commitment transactions ALWAYS signed via ATECC608A cold path
2. ✅ **No Key Export**: Private key never leaves ATECC608A chip
3. ✅ **Signing Rate Limit**: ATECC608A supports configurable signing rate limits (future)
4. ✅ **LDK Watchtower**: Channel state monitored by watchtower, revoked states published as penalty

**Residual Risk**: LOW
- Attacker cannot extract private key to sign offline
- Old state broadcast detected by watchtower
- Penalty transaction recovers funds

**Detection**:
- LDK watchtower: Monitors blockchain for revoked commitment transactions
- Signing audit log: All ATECC608A operations logged with HTLC metadata

---

### T6: Watchdog Bypass (Crash without Detection)

**Attack Vector**:
Attacker crashes node backend but keeps watchdog heartbeat alive to prevent systemd from restarting the process. ~~ESP32 reset DROPPED~~ — watchdog now via systemd `WatchdogSec=60`.

**Mitigations**:
1. ✅ **Heartbeat in Backend**: Watchdog heartbeat sent by node backend via `sd_notify("WATCHDOG=1")`
2. ✅ **Timeout Enforcement**: Systemd `WatchdogSec=60` restarts process if no heartbeat within timeout. ~~ESP32 hardware reset~~ DROPPED.
3. ✅ **Health Aggregation**: HealthAggregator checks DB + LDK subsystem health before sending heartbeat. Withholds `sd_notify` if any probe reports Degraded.

**Residual Risk**: MEDIUM
- Separate malicious process could spoof `sd_notify` while backend is crashed
- **Mitigation**: `tee-watchdog.service` runs as dedicated user; only that process can send `sd_notify`

**Detection**:
- Missing heartbeats: systemd journal logs restart event (~~ESP32 gatekeeper UART DROPPED~~)
- External monitoring: Node offline alerts

---

### T7: I2C Bus Sniffing (Key Extraction via Bus Monitor)

**Attack Vector**:
Attacker with physical access attaches I2C bus analyzer to sniff signing operations and extract private key material.

**Mitigations**:
1. ✅ **Key Never Transmitted**: ATECC608A private key never leaves chip, only signatures transmitted
2. ✅ **Encrypted I2C (ATECC608A Feature)**: Optional I2C encryption with session keys (not implemented)
3. ❌ **Physical Tamper Detection**: Requires tamper-evident casing (out of scope)

**Residual Risk**: HIGH (if physical access assumed)
- Attacker can observe signatures, but cannot extract private key from signatures alone
- **Future**: Enable ATECC608A encrypted I2C for defense-in-depth

**Detection**:
- Physical inspection: Detect bus analyzer attachment
- Tamper-evident seals (future hardware revision)

---

### T8: Remote Attestation Spoofing

**Attack Vector**:
Compromised node claims valid boot measurements to remote peers, even though kernel is tampered.

**Mitigations**:
1. ✅ **Nonce-Based Challenge**: Remote node sends fresh nonce, compromised node cannot replay old attestation
2. ✅ **TPM PCR Inclusion**: Attestation includes all PCR values, not just PCR-10
3. ✅ **Signature Verification**: Remote node verifies attestation signature against known ATECC608A public key
4. ⚠️ **Public Key Distribution**: ATECC608A public keys distributed via trusted channel (manual for now)

**Residual Risk**: MEDIUM
- Attacker could provision malicious ATECC608A with known private key
- **Mitigation**: Public key registry with certificate pinning (future)

**Detection**:
- Public key mismatch: Remote node rejects unknown public keys
- PCR anomaly detection: Machine learning on expected PCR values

---

### T9: L402 Macaroon Root Key Theft (Database Compromise)

**Attack Vector**:
Attacker gains read access to the SQLite database (e.g. via SQL injection or stealing the SD card) and extracts the L402 Macaroon Root Key. With this key, they can forge unlimited valid macaroons for free API usage.

**Mitigations**:
1. ✅ **Hardware Wrapping Key**: The L402 root key is encrypted with AES-256-GCM using a wrapping key derived from ATECC608A (Slot 1).
2. ✅ **Secure Memory Decryption**: On boot, the database reads the encrypted root key, ATECC608A unwraps it, and it is stored in `SecureMemory` (RAM, mlocked, zeroize on drop).
3. ✅ **No Plaintext Persistence**: The root key never exists in plaintext on disk.
4. ✅ **Disk Theft Protection**: Without the physical ATECC608A chip, stealing the `lightning.db` yields only a worthless cipher blob.

**Residual Risk**: LOW
- Requires memory dumping to steal the hot key from `SecureMemory`.
- Note: This is an API consumption threat (stealing compute/bandwidth), not a fund theft threat.

**Detection**:
- Unusually high volume of Macaroon validations not matching top-up records.

---

### T10: LDK State Access via AI Agent Pipeline

**Attack Vector**:
An AI Agent receives a malicious prompt (Prompt Injection) containing attack vectors designed to manipulate the backend process.
- **Vector A (Semantic Access)**: The LLM attempts to call internal functions or tools to access the LDK Node.
- **Vector B (True RCE)**: The prompt payload triggers a memory-corruption vulnerability in a Rust dependency, allowing arbitrary code execution within the Node's memory space.

**Mitigations**:
1. ✅ **Seal Semantic Access (Vector A Mitigation - M5.3)**: `AgentExecutor` and its tools drop all `Arc<ldk_node::Node>` dependencies. The LLM simply has no accessible tools or references to poke LDK memory.
2. ✅ **Hardware Key Isolation (Vector B Partial - M4 `The Vault`)**: If an attacker achieves True RCE (Vector B), they can access the heap (including `AppState` copies). However, ATECC608A makes **private key extraction** impossible.
3. ⚠️ **Process Sandbox (`M7 Future`)**: AI executions move out of the Node memory space into WASM sandboxes or isolated processes with strict IPC.

**Critical Limitation — Blind Signer Paradox (Vector B)**:
ATECC608A is a **blind signer**: it receives a hash, returns a signature. It has no RAM to maintain Lightning channel state machines. It CANNOT validate whether a `CommitmentTransaction` is legitimate or malicious. If an attacker achieves True RCE (Vector B), they do NOT need to extract keys — they can craft a malicious commitment transaction (e.g., sweep 100% channel capacity to attacker address via inflated fees or fake HTLCs) and instruct ATECC608A to sign it. The chip will comply blindly.

**ATECC608A protects against**: Key Exfiltration (T2 memory dump)
**ATECC608A does NOT protect against**: Unauthorized Signing (T10 Vector B RCE)

To protect against unauthorized signing, a **Validating Signer** (VLS pattern with full channel state verification inside a secure enclave) or **WASM Sandbox** (M7) isolating the AI agent's execution from the signing path is required.

**Residual Risk**: MEDIUM (for True RCE until WASM Sandbox is implemented)
- Vector A (Tool exploitation) risk is eliminated (reduced to 0).
- Vector B (RCE) inside Rust is statistically very unlikely due to memory safety guarantees.
- Even with RCE, attacker cannot extract keys (only sign with them) — limits attack to one-time theft vs persistent compromise.
- Prompt injection cannot execute arbitrary remote code without an underlying library vulnerability.

**Detection**:
- Watchdog failure or backend panics due to unexpected memory state.
- Channel state anomaly detection (unexpected commitment updates).

---

## Known Limitations of Hardware Security (ATECC608A)

The following are fundamental limitations of the current hardware security architecture. They are NOT bugs — they are documented trade-offs inherent to using a Secure Element without a full Trusted Execution Environment.

| Limitation | Impact | Scope of Fix | Status |
|------------|--------|--------------|--------|
| **Blind Signer** | ATECC608A signs any hash without validating channel state. RCE attacker can forge malicious commitment transactions. | VLS (Validating Signer) in OP-TEE TA (Phase 3) | OPEN |
| **BDK On-chain Wallet Hot** | Same seed derives both Lightning keys and BDK wallet. `xprv` root key in RAM for on-chain ops. | BDK Hardware Signer (PSBT via hardware) — Phase 3 scope | OPEN |
| **NodeSigner in RAM** | Node identity key (`node_secret_key`) exposed in RAM. Enables gossip impersonation, invoice forgery, onion routing decryption. | OP-TEE Trusted Application (Phase 2/3) | OPEN |
| **Plaintext SQLite (channel data)** | Channel monitors, balances, routing data stored unencrypted. Physical access exposes financial history. | SQLCipher — post LUKS2 already mitigates block-level | LOW |
| **Blocking I2C Signing** | `EcdsaChannelSigner` is synchronous. I2C latency (50-100ms) blocks LDK threads. | Dedicated thread pool in Phase 2 `TeeChannelSigner` | OPEN (Phase 2) |
| ~~**Seed Memory Remnants**~~ | ~~`seed_bytes: [u8; 64]` not wrapped in `Zeroizing<>`~~ | ✅ **FIXED** — `Zeroizing<[u8; 64]>` in ldk_manager.rs (P1-1) | **DONE** |
| ~~**Seed Plaintext on Disk**~~ | ~~`signer_seed.hex` readable on disk~~ | ✅ **FIXED** — AES-256-GCM encrypted (T-403), LUKS2 block encryption (T-801) | **DONE** |
| ~~**L402 Key Plaintext in DB**~~ | ~~`macaroon_root_key` readable in SQLite~~ | ✅ **FIXED** — AES-256-GCM encrypted with `enc:` prefix (T-501) | **DONE** |
| ~~**Nonce Reuse Risk**~~ | ~~AES-GCM static IV risk~~ | ✅ **FIXED** — `OsRng` per-encryption nonce in `seed_encryption.rs` | **DONE** |
| ~~**KDF Entropy**~~ | ~~Raw SHA256 insufficient for key derivation~~ | ✅ **FIXED** — HKDF-SHA256 with domain-separated info strings | **DONE** |

### Hardware Role Clarification

- **ATECC608A** = Root of Trust (mandatory). All crypto operations (ECDSA sign, AES wrap, KDF) go directly to this chip via I2C from Radxa.
- ~~**ESP32**~~ = **DROPPED** (2026-02-25). Was previously a bridge/gatekeeper. Radxa now communicates directly with ATECC608A and TPM via I2C bus. Simplifies architecture and reduces attack surface.

---

## Security Boundaries

### Trust Boundaries

```
┌─────────────────────────────────────────────────────────────┐
│                    TRUST BOUNDARIES                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌────────────────────────────────────────────────────┐    │
│  │  UNTRUSTED ZONE (Normal World)                     │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │  Linux Kernel (Debian)                       │  │    │
│  │  │  - Can be compromised                        │  │    │
│  │  │  - Read-only root mitigates persistence     │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │  Node Backend (Rust)                         │  │    │
│  │  │  - Runs as non-root user                     │  │    │
│  │  │  - Can be exploited                          │  │    │
│  │  │  - Cannot extract hardware keys              │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  └────────────────────────────────────────────────────┘    │
│                         │                                    │
│                         │ I2C (signing requests only)        │
│                         ▼                                    │
│  ┌────────────────────────────────────────────────────┐    │
│  │  TRUSTED ZONE (Secure World)                       │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │  Systemd Watchdog + OP-TEE (future)          │  │    │
│  │  │  - WatchdogSec=60 + sd_notify                │  │    │
│  │  │  - HealthAggregator (DB + LDK probes)        │  │    │
│  │  │  - ~~ESP32 Gatekeeper~~ DROPPED              │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │  ATECC608A (Crypto Co-processor)             │  │    │
│  │  │  - Private key NEVER exported                │  │    │
│  │  │  - Signing operations only                   │  │    │
│  │  │  - Hardware-locked after provisioning        │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │  TPM 2.0 (Trusted Platform Module)           │  │    │
│  │  │  - Boot measurements (PCR extend)            │  │    │
│  │  │  - Tamper-evident                            │  │    │
│  │  │  - Cannot be rolled back                     │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  └────────────────────────────────────────────────────┘    │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Attack Surface Analysis

| Component | Attack Surface | Mitigations |
|-----------|----------------|-------------|
| **Frontend (React)** | XSS, CSRF, client-side tampering | CSP headers, React sanitization, JWT auth |
| **Backend (Rust)** | API exploits, injection, auth bypass | ValidatedJson, L402, JWT refresh |
| **Database (SQLite)** | SQL injection, unauthorized access | Diesel ORM, parameterized queries |
| **Lightning Network** | Channel jamming, routing attacks | LDK built-in mitigations, watchtower |
| **WebSocket** | DoS, message injection | Rate limiting, message validation |
| ~~**UART (ESP32 ↔ Radxa)**~~ | ~~Eavesdropping, MITM~~ | **DROPPED** — ESP32 removed. No UART in architecture. |
| **I2C (ATECC608A/TPM)** | Bus sniffing, replay attacks | Encrypted I2C (future), nonce-based challenges |
| **Filesystem** | Malware persistence, config tampering | Read-only root, zRAM overlay |

## Compliance and Certification

### Future Certification Targets

| Standard | Scope | Timeline |
|----------|-------|----------|
| **FIPS 140-3 Level 2** | ATECC608A crypto module | Post-Milestone 4 |
| **Common Criteria EAL4+** | Full system evaluation | 2027 (if demand exists) |
| **TCG Trusted Boot** | Measured boot compliance | Milestone 3 |

### Audit Recommendations

**Internal Audits** (every release):
- Code review: Constitution compliance, security patterns
- Dependency scanning: Known vulnerabilities (Cargo audit)
- Fuzzing: API endpoints, WebSocket handlers

**External Audits** (annually):
- Penetration testing: Network, API, Lightning Network
- Hardware security review: I2C bus, physical tamper resistance (~~UART DROPPED~~)
- Cryptographic implementation review: Signing logic, key management

### Cross-Validation Findings (2026-02)

Codebase audit cross-referenced against security model. Results:

| Finding | Verdict | Detail |
|---------|---------|--------|
| L402 `macaroon_root_key` volatile (lost on restart) | **FALSE ALARM** | Key persisted in SQLite `lightning.db`. L402 Macaroons survive reboots. |
| `EVENT_LISTENER` not enforced (events ignored) | **FALSE ALARM** | Audit confirmed EventBus pattern is used and listeners are registered at bootstrap. |
| Prompt Injection → RCE → LDK Memory Dump | **CONFIRMED** | `AgentExecutor` held `Arc<ldk_node::Node>`, giving AI agent access to LDK memory. **Mitigated in M5.3**: removed `Arc<ldk_node::Node>` dependency, breaking the attack chain. |
| Native App isolation (shared memory space) | **CONFIRMED (theoretical)** | Built-in apps share process memory with backend. Lower priority — requires `cdylib` + process isolation or WASM sandbox. Tracked as post-M6 enhancement. |
| Third-party "Reboot Bypass" claim | **FALSE ALARM** | LDK does NOT serialize signers to disk (writes `0u32` since v0.0.113). `read_chan_signer` is legacy-only. No bypass possible via reboot. |

## Incident Response

### Attack Detection

**Automated Monitoring**:
- Failed boot attempts (TPM PCR mismatch — ~~ESP32 logs DROPPED~~)
- Anomalous PCR values (remote attestation)
- I2C latency spikes (bus contention or attack)
- Signing rate anomalies (ATECC608A audit log)
- Watchdog timeouts (system crashes)

**Manual Inspection**:
- Quarterly filesystem integrity checks
- Annual hardware inspection (tamper seals, bus connections)

### Response Procedures

**Severity Levels**:

| Level | Criteria | Response Time | Actions |
|-------|----------|---------------|---------|
| **CRITICAL** | Key compromise suspected | Immediate | Kill system (systemd stop + power off), notify all peers, rotate keys |
| **HIGH** | Boot integrity failure | < 1 hour | Investigate boot logs, restore from backup, re-attest |
| **MEDIUM** | Anomalous signing patterns | < 24 hours | Audit signing logs, check HTLC thresholds, rotate hot keys |
| **LOW** | Watchdog timeout | < 1 week | Review backend logs, check for resource exhaustion |

### Recovery Procedures

**Scenario: Compromised Backend Binary**

1. **Detection**: Remote attestation fails (PCR mismatch)
2. **Isolation**: Backend detects TPM PCR mismatch, enters recovery mode
3. **Analysis**: Compare PCR values with known-good baseline
4. **Remediation**: Re-flash SD card with verified image
5. **Verification**: Re-attest with multiple remote peers
6. **Restoration**: Restore Lightning channel state from `/var/lib/ldk` backup

**Scenario: ATECC608A Key Compromise (Theoretical)**

1. **Detection**: Unauthorized transactions signed (impossible without physical attack)
2. **Response**: THIS SHOULD NEVER HAPPEN (key never exported)
3. **Mitigation**: If detected, assume hardware tampering, replace device
4. **Recovery**: Close all channels, migrate to new hardware, re-establish channels

## Security Roadmap

### Milestone 1: Foundation (Week 1-2)
- ✅ Mock implementations for testing
- ✅ Security code review of trait abstractions
- ✅ Dependency audit (Cargo audit)

### Milestone 2: Debian Hardening (Week 2-3)
- ✅ Read-only root filesystem
- ✅ zRAM overlay configuration
- ✅ SELinux/AppArmor policies (future)

### Milestone 3: Hardware Integration (Week 4-6)
- ⏳ ~~ESP32 firmware~~ DROPPED → Direct Radxa↔ATECC608A I2C provider
- ⏳ ATECC608A key provisioning process
- ⏳ TPM PCR baseline measurement

### Milestone 4: The Vault (Phase 1 Complete)
- ✅ HybridSigner enum + TeeSignerFactory in LDK fork (`our LDK fork`)
- ✅ Seed `Zeroizing<[u8;64]>` — memory cleared on drop (P1-1)
- ✅ WrapKeyProvider trait + MockWrapKeyProvider (P1-2)
- ✅ Seed AES-256-GCM encrypted at rest (T-403, P1-3)
- ⏳ Phase 2 (Radxa + OP-TEE): `OpteeWrapKeyProvider` + Key Manager TA — blocked on R-012

### Milestone 5: The Gateway & Sandbox (Phase 1 Complete)
- ✅ L402 macaroon root key AES-256-GCM encrypted in SQLite (T-501, P1-4)
- ✅ M5.3: AgentExecutor decoupled from LDK memory
- ⏳ L402 → Keysend protocol (Hieu)

### Milestone 8: Physical Security (Phase 1 Complete)
- ✅ LUKS2 full-disk encryption for `/var/lib/ldk/` (T-801, P1-5)
- ✅ Passphrase: `/root/luks_passphrase` (mode 0600) — Phase 1
- ⏳ Phase 2 (Radxa + OP-TEE): PCR-sealed passphrase — blocked on R-012

### Milestone 7: Production Hardening (Future)
- 🔄 Encrypted I2C for ATECC608A
- 🔄 Transparency log for firmware updates
- 🔄 Full SQLite encryption
- 🔄 WASM AI Sandbox
- 🔄 Tamper-evident hardware packaging

---

**See Also**:
- [ARCHITECTURE.md](ARCHITECTURE.md) - System design and integration
- [ROADMAP.md](ROADMAP.md) - Implementation timeline
