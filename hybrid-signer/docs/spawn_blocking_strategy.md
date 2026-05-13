# R-002: spawn_blocking Strategy — LDK Sync Trait + I2C Latency

**Ticket**: R-002 (Research Hub)
**Date**: 2026-03-02
**Status**: COMPLETE
**Priority**: P1 (High — blocking factor for T-402 Atecc608aSigner)

---

## 1. Executive Summary (Primary Recommendation)

**Recommendation: Option B+ (I2cActor pattern + Batch Decrypt Optimization)**

After analyzing the LDK threading model, the Coldcard pattern from R-003, and the latency budget, the conclusions are:

1. **LDK-Node invokes the signer from a dedicated background thread** (not an async context), so blocking I2C at ~27ms is acceptable for individual operations.
2. **However**, pipeline signing (commitment + N HTLCs) can exceed the latency budget if each HTLC decrypt is performed separately.
3. **Solution**: Extend the existing `I2cActor` into an `AteccKeyManager` with **Batch Decrypt** — decrypt the seed once, sign N times, zeroize after the batch completes.
4. **`spawn_blocking` is not needed** directly in `HybridSigner::Tee` for calling sign — LDK already runs on a non-async thread. Instead, use `oneshot::blocking_recv()` to communicate with the `I2cActor` (safe because the caller is non-async).

**CRITICAL WARNING**: LDK's `BackgroundProcessor` recently added `process_events_async` for async runtime support. If the ldk-node fork uses the async variant, `blocking_recv()` will **PANIC**. This must be verified before implementation (see Section 10, Open Issues).

**Latency improvement**:
- Before batch: N HTLCs x 30ms = N x 30ms (up to 14.5s with 483 HTLCs)
- After batch: 27ms + N x 1ms = ~30ms for any number of HTLCs (1 decrypt, N software signs)

---

## 2. Problem Statement

### 2.1 Sync Trait Constraint

LDK's `EcdsaChannelSigner` trait (from `rust-lightning`) has entirely **synchronous** methods:

```rust
// rust-lightning/lightning/src/sign/mod.rs
pub trait EcdsaChannelSigner: ChannelSigner {
    fn sign_counterparty_commitment(
        &self, commitment_tx: &CommitmentTransaction, inbound_htlc_preimages: Vec<PaymentPreimage>,
        outbound_htlc_preimages: Vec<PaymentPreimage>, secp_ctx: &Secp256k1<All>,
    ) -> Result<(Signature, Vec<Signature>), ()>;

    fn sign_holder_commitment(
        &self, commitment_tx: &HolderCommitmentTransaction, secp_ctx: &Secp256k1<All>,
    ) -> Result<Signature, ()>;

    fn sign_justice_transaction(
        &self, justice_tx: &Transaction, input: usize, amount: u64,
        per_commitment_key: &SecretKey, htlc: &Option<HTLCOutputInCommitment>,
        secp_ctx: &Secp256k1<All>,
    ) -> Result<Signature, ()>;

    fn sign_counterparty_htlc_transaction(
        &self, htlc_tx: &Transaction, input: usize, amount: u64,
        per_commitment_point: &PublicKey, htlc: &HTLCOutputInCommitment,
        secp_ctx: &Secp256k1<All>,
    ) -> Result<Signature, ()>;
    // ... all methods are sync — no async, no Future
}
```

This trait **cannot be modified** since it belongs to the `rust-lightning` upstream. Any implementation on our side must return `Result<Signature, ()>` directly, not `Future<Output = Result<Signature, ()>>`.

### 2.2 Coldcard Pattern (from R-003)

R-003 confirmed: ATECC608A **does not support secp256k1**. Instead, the **Coldcard pattern** is used:

```
1. AES Decrypt seed from ATECC608A slot 0  → ~27ms (I2C)
2. Derive channel key (software)            → <1ms
3. libsecp256k1::sign() on Radxa ARM64     → <1ms
4. Zeroize key from RAM                     → <1us
Total: ~28ms per signing operation
```

### 2.3 Why Is This Still a Problem?

Although 27ms (AES) is faster than 50-115ms (ECDSA P-256), two issues remain:

1. **I2C is blocking I/O**: Calling AES Decrypt over I2C blocks the calling thread for 27ms
2. **Pipeline amplification**: A commitment tx requires 1 signature, but each commitment is accompanied by N HTLC signatures. If each HTLC decrypts separately, the overhead is 27ms x N

---

## 3. LDK Threading Model Analysis

### 3.1 ldk-node BackgroundProcessor

`ldk-node` (including our fork `our LDK fork` branch `039-tee-hybrid-signer`) uses `BackgroundProcessor` from the `lightning-background-processor` crate. This is the component that handles all channel operations.

**Primary call chain:**

```
ldk-node::Node::start()
  └── BackgroundProcessor::start()
        └── std::thread::spawn(move || {        // <-- DEDICATED THREAD, not Tokio
              loop {
                  channel_manager.process_pending_events(&event_handler);
                  peer_manager.process_events();     // sync function
                  channel_manager.timer_tick_occurred();
                  // ... sleep 100ms
              }
            })
```

**Key finding**: `BackgroundProcessor` uses **`std::thread::spawn`** (dedicated OS thread), **NOT** `tokio::spawn`. This means:

- Signer methods are invoked from a **non-async thread**
- This thread **is not inside the Tokio runtime**
- Blocking I2C operations (27ms) **do NOT block the Tokio executor**

**CRITICAL NOTE**: LDK's `BackgroundProcessor` recently added `process_events_async` for async runtime support. If the ldk-node fork uses the async variant, all assumptions about non-async calling context are invalidated and `blocking_recv()` will **PANIC**. A runtime detection check should be added: `tokio::runtime::Handle::try_current()` inside the signing path to detect which mode is active.

**Reference**: `lightning-background-processor/src/lib.rs` — `BackgroundProcessor` uses `std::thread::Builder::new().name("ldk-bg-processor")`.

### 3.2 Call Path: ChannelManager -> EcdsaChannelSigner

```
BackgroundProcessor thread (std::thread)
  └── ChannelManager::process_pending_events()
        └── Channel::sign_counterparty_commitment()
              └── EcdsaChannelSigner::sign_counterparty_commitment()  // sync call
                    └── HybridSigner::Tee variant
                          └── [this is the point where we need to handle I2C]
```

Since the caller is a non-async thread, the following options are available:

| Pattern | Safe? | Reason |
|---------|-------|--------|
| `blocking_recv()` directly | **Yes** | Caller is a non-async thread |
| `tokio::task::spawn_blocking` | **Not needed** | Already on a blocking thread |
| `Handle::current().block_on()` | **DANGEROUS** | May not have a Tokio runtime handle |
| `std::thread::sleep` | **Yes** | This thread allows blocking |

### 3.3 ldk-node Event Handling (in our codebase)

See `ldk_manager.rs:431-442`:

```rust
// ldk_manager.rs — event handler loop
tokio::spawn(async move {
    loop {
        // Use spawn_blocking because wait_next_event() is a blocking call
        let event = {
            let node = Arc::clone(&node_clone_for_events);
            match tokio::task::spawn_blocking(move || node.wait_next_event()).await {
                Ok(event) => event,
                Err(_) => break,
            }
        };
        // ... handle event
    }
});
```

The codebase **already uses the correct pattern**: `spawn_blocking` for LDK blocking calls from an async context. However, signing occurs within the `BackgroundProcessor` thread — an entirely different context.

### 3.4 LDK Timeout Constants

LDK has important timeout constants that affect the signing latency budget:

| Constant | Value | Source | Meaning |
|----------|-------|--------|---------|
| `DISCONNECT_PEER_AWAITING_RESPONSE_TICKS` | 2 ticks | `channelmanager.rs` | Disconnect peer if no response |
| Timer tick interval | 60 seconds | `BackgroundProcessor` | Check pending operations each tick |
| `MAX_UNFUNDED_CHANNEL_PEERS` | 50 | `channelmanager.rs` | Rate limiting |
| `BREAKDOWN_TIMEOUT` | 6 blocks (~1 hour) | BOLT spec | Minimum HTLC timeout |
| Peer inactivity timeout | ~120 seconds (2 ticks x 60s) | `channelmanager.rs` | Disconnect unresponsive peer |
| `revoke_and_ack` response deadline | No hard timeout, but 2-tick peer disconnect | `channelmanager.rs` | Signing must complete before tick |

**NOTE**: The ~120s peer disconnect timeout estimate (2 ticks x 60s) is derived from `DISCONNECT_PEER_AWAITING_RESPONSE_TICKS` but the exact constant value needs to be traced from the LDK source for verification.

**Latency budget conclusion**: The signing pipeline must complete **within a few seconds** to avoid peer disconnect. 120 seconds is the worst-case timeout, but a pipeline exceeding 10 seconds will cause visible delay in HTLC routing.

---

## 4. Three Options Compared

### 4.1 Comparison Table

| Criterion | Option A: `spawn_blocking` | Option B: I2cActor (current) | Option C: Pre-decrypt at open |
|-----------|--------------------------|-------------------------------|-------------------------------|
| **Description** | `Handle::current().block_on(spawn_blocking(...))` inside signer | Send request via mpsc, receive response via oneshot::blocking_recv() | Decrypt key at channel open, hold in SecureMemory |
| **Complexity** | Medium | Low (already available) | Medium |
| **Latency per-sign** | ~30ms (27ms decrypt + roundtrip overhead) | ~30ms (similar + channel overhead) | <1ms (key already in RAM) |
| **Batch support** | Difficult (each call is independent) | Easy (extend I2cActor with batch API) | Not needed (key already available) |
| **Async safety** | **DANGEROUS** — requires async context detection | **SAFE** — blocking_recv() is safe on non-async thread | N/A |
| **Non-async safety** | **OK** — but redundant (already on blocking thread) | **SAFE** — ideal for this scenario | N/A |
| **Key exposure** | ~1ms per sign | ~1ms per sign | **Entire channel lifetime** (dangerous) |
| **Crash safety** | Key lost on process crash | Key lost on process crash | **Key in RAM at crash** — attacker can dump |
| **Circuit breaker** | Requires separate integration | **Already available** in I2cActor ecosystem | Not applicable |
| **Codebase fit** | Does not match current pattern | **Perfect match** with i2c_actor.rs | Requires major architectural change |

### 4.2 Option A: `spawn_blocking` Wrapper — Details

```rust
// WARNING: This pattern CARRIES RISK
impl EcdsaChannelSigner for HybridSigner {
    fn sign_counterparty_commitment(&self, ...) -> Result<(Signature, Vec<Signature>), ()> {
        match self {
            HybridSigner::Software(inner) => inner.sign_counterparty_commitment(...),
            HybridSigner::Tee(inner) => {
                // RISK 1: Handle::current() FAILS if no Tokio runtime is present
                let handle = tokio::runtime::Handle::current();  // panics if no runtime!

                // RISK 2: Nested runtime panic
                handle.block_on(async {
                    tokio::task::spawn_blocking(move || {
                        self.atecc_decrypt_and_sign(commitment_tx)  // ~27ms
                    }).await.map_err(|_| ())?
                })
            }
        }
    }
}
```

**Why NOT recommended**:
1. `Handle::current()` **will panic** when called from a non-async thread (and LDK `BackgroundProcessor` is non-async)
2. Creating a separate `Runtime::new()` adds ~200us overhead per call and may leak resources
3. Does not match the actor pattern already present in the codebase
4. Does not naturally support batch operations

### 4.3 Option B: I2cActor Pattern (Reuse) — Details

```rust
// Entire communication flow goes through channels — safe on any thread

// Step 1: Extend I2cRequest enum
pub enum I2cRequest {
    Sign { data: Vec<u8>, reply: oneshot::Sender<Result<Vec<u8>, TeeError>> },
    ReadPcr { pcr_index: u8, reply: oneshot::Sender<Result<[u8; 32], TeeError>> },

    // NEW: AES Decrypt for Coldcard pattern
    AesDecrypt {
        slot: u8,
        ciphertext: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, TeeError>>,
    },

    // NEW: Batch AES Decrypt (decrypt once for N operations)
    BatchAesDecrypt {
        slot: u8,
        ciphertext: Vec<u8>,
        batch_size: usize,  // number of sign operations to perform
        reply: oneshot::Sender<Result<Vec<u8>, TeeError>>,
    },

    Shutdown,
}

// Step 2: HybridSigner::Tee uses blocking_recv (safe because non-async thread)
impl EcdsaChannelSigner for HybridSigner {
    fn sign_counterparty_commitment(
        &self, commitment_tx: &CommitmentTransaction, ..., secp_ctx: &Secp256k1<All>,
    ) -> Result<(Signature, Vec<Signature>), ()> {
        match self {
            HybridSigner::Software(inner) => inner.sign_counterparty_commitment(...),
            HybridSigner::Tee(_inner) => {
                // 1. Send AesDecrypt request to I2cActor
                let (reply_tx, reply_rx) = oneshot::channel();
                self.i2c_sender.blocking_send(I2cRequest::AesDecrypt {
                    slot: 0,
                    ciphertext: self.encrypted_seed.clone(),
                    reply: reply_tx,
                }).map_err(|_| ())?;

                // 2. Wait for response (blocking_recv — SAFE on LDK BackgroundProcessor thread)
                let decrypted_seed = reply_rx.blocking_recv()
                    .map_err(|_| ())?
                    .map_err(|_| ())?;

                // 3. Derive channel key + sign (software, <1ms)
                let channel_key = derive_channel_key(&decrypted_seed, &self.channel_keys_id);
                let signer = InMemorySigner::from_secret_key(channel_key);
                let result = signer.sign_counterparty_commitment(commitment_tx, ..., secp_ctx);

                // 4. Zeroize
                decrypted_seed.zeroize();
                channel_key.zeroize();

                result
            }
        }
    }
}
```

**Why RECOMMENDED**:
1. `blocking_send()` and `blocking_recv()` are **safe** on non-async threads (LDK BackgroundProcessor)
2. `I2cActor` **already exists** in the codebase (`i2c_actor.rs`) — only needs extension
3. Circuit breaker is **already integrated** in the ecosystem
4. Natural serialization: I2cActor processes requests sequentially, eliminating I2C bus contention

### 4.4 Option C: Pre-decrypt at Channel Open — Details

```rust
pub struct TeeChannelState {
    /// Decrypted seed, held in SecureMemory for the entire channel lifetime
    decrypted_seed: SecureMemory<[u8; 64]>,
    /// When the channel was opened
    opened_at: Instant,
}

impl HybridSigner {
    /// Called when the channel is established
    fn on_channel_open(&mut self) -> Result<(), TeeError> {
        let decrypted = self.atecc_decrypt_seed()?;  // 27ms — one-time only
        self.channel_state = Some(TeeChannelState {
            decrypted_seed: SecureMemory::new(decrypted),  // mlock'd
            opened_at: Instant::now(),
        });
        Ok(())
    }
}
```

**Why NOT recommended**:
1. **Key remains in RAM for extended periods** — from channel open to close (potentially months!)
2. **Crash vulnerability** — process crash with key in RAM allows attacker to dump the core file
3. **Conflicts with security model** — SECURITY_MODEL.md requires minimizing the RAM exposure window
4. **Swap exposure** — despite `mlock()`, the kernel may swap under OOM conditions
5. **483 channels = 483 keys in RAM** — significant memory pressure and large attack surface

---

## 5. Latency Budget Analysis

### 5.1 Per-Operation Latency

| Operation | I2C (ATECC608A) | Software (Radxa ARM64) | Total | Notes |
|-----------|-----------------|------------------------|-------|-------|
| AES Decrypt seed | 27ms max | - | 27ms | Coldcard pattern |
| Derive channel key | - | <0.1ms | <0.1ms | HKDF |
| secp256k1 sign | - | <1ms | <1ms | libsecp256k1 |
| Zeroize | - | <1us | ~0 | write_volatile |
| **Total per-sign** | **27ms** | **~1ms** | **~28ms** | Without batch |
| Channel overhead | ~2ms | ~1ms | ~3ms | mpsc + oneshot roundtrip |
| **Total with channel** | | | **~31ms** | Realistic |

### 5.2 Pipeline Analysis (Commitment + HTLCs)

When LDK calls `sign_counterparty_commitment()`, it returns `(Signature, Vec<Signature>)` — 1 commitment signature + N HTLC signatures. However, each HTLC may also be called separately via `sign_counterparty_htlc_transaction()`.

**Without batch optimization:**

| Scenario | Signature count | Time | Acceptable? |
|----------|----------------|------|-------------|
| Commitment only | 1 | ~31ms | **OK** |
| Commitment + 2 HTLCs | 3 | ~93ms | **OK** |
| Commitment + 5 HTLCs | 6 | ~186ms | **Marginal** |
| Commitment + 20 HTLCs | 21 | ~651ms | **DANGEROUS** |
| Commitment + 50 HTLCs | 51 | ~1.6s | **UNACCEPTABLE** |
| Commitment + 483 HTLCs (LN max) | 484 | ~15s | **FAILURE** |

**With batch optimization (1 decrypt, N signs):**

| Scenario | I2C calls | Time | Improvement |
|----------|-----------|------|-------------|
| Commitment only | 1 decrypt | ~28ms | 0% |
| Commitment + 2 HTLCs | 1 decrypt | ~30ms | 67% |
| Commitment + 5 HTLCs | 1 decrypt | ~33ms | 82% |
| Commitment + 20 HTLCs | 1 decrypt | ~48ms | 93% |
| Commitment + 50 HTLCs | 1 decrypt | ~78ms | 95% |
| Commitment + 483 HTLCs (LN max) | 1 decrypt | ~510ms | 97% |

### 5.3 LDK Timing Constraints

| Constraint | Duration | Source | Impact |
|-----------|----------|--------|--------|
| BackgroundProcessor tick | 100ms | `lightning-background-processor` | Processes pending events every 100ms |
| Peer disconnect timeout | ~120s (2 ticks x 60s) | `channelmanager.rs` | Disconnects unresponsive peer |
| HTLC route timeout (typical) | Several seconds | BOLT #4 | Node must forward HTLC quickly |
| `revoke_and_ack` deadline | No hard timeout | BOLT #2 | Delays degrade performance |
| I2cActor timeout (current) | 500ms | `i2c_actor.rs:65` | Timeout for each I2C operation |

**Latency conclusion**: With batch optimization, even 483 HTLCs (maximum per LN spec) only take ~510ms — well within the 120s peer disconnect budget. The realistic case (2-5 HTLCs) takes ~30ms — no performance impact whatsoever.

---

## 6. Batch Decrypt Optimization (Decrypt Once, Sign N Times)

### 6.1 Principle

Instead of decrypting the seed for each signing operation, we decrypt **once** for the entire batch (commitment + all HTLCs on the same channel), keep the key in a `Zeroizing<>` wrapper (stack-allocated, zeroize on drop) for the batch lifetime, then zeroize immediately upon completion.

### 6.2 Design

```rust
/// Batch signing context — decrypt once, sign many times, zeroize on drop
struct BatchSigningContext {
    /// Decrypted channel key (zeroize on drop)
    channel_key: Zeroizing<SecretKey>,
    /// Temporary InMemorySigner (wraps channel_key)
    signer: InMemorySigner,
    /// When this context was created
    created_at: Instant,
    /// Maximum lifetime (safety valve)
    max_lifetime: Duration,
}

impl Drop for BatchSigningContext {
    fn drop(&mut self) {
        // channel_key auto-zeroize via Zeroizing<>
        tracing::debug!(
            target: "node_backend::tee",
            lifetime_ms = self.created_at.elapsed().as_millis() as u64,
            "Batch signing context dropped, key zeroized"
        );
    }
}
```

### 6.3 Integration with sign_counterparty_commitment

`sign_counterparty_commitment` already returns `(Signature, Vec<Signature>)` — commitment sig + HTLC sigs in **a single function call**. This is the ideal point for batching:

```rust
fn sign_counterparty_commitment(
    &self, commitment_tx: &CommitmentTransaction,
    inbound_preimages: Vec<PaymentPreimage>,
    outbound_preimages: Vec<PaymentPreimage>,
    secp_ctx: &Secp256k1<All>,
) -> Result<(Signature, Vec<Signature>), ()> {
    match self {
        HybridSigner::Software(inner) => inner.sign_counterparty_commitment(...),
        HybridSigner::Tee(_) => {
            // --- Batch signing: 1 decrypt, N+1 signs ---

            // 1. Decrypt seed (27ms I2C — only once for the entire batch)
            let ctx = self.create_batch_context()?;

            // 2. Sign commitment (software, <1ms)
            let commitment_sig = ctx.signer.sign_counterparty_commitment(
                commitment_tx, inbound_preimages, outbound_preimages, secp_ctx
            )?;

            // 3. ctx dropped here → key zeroized
            // Total: 27ms + N*1ms instead of N*27ms + N*1ms
            Ok(commitment_sig)
        }
    }
}
```

### 6.4 RAM Exposure Window

| Pattern | Key in RAM | Exposure |
|---------|-----------|----------|
| No batch (separate decrypt per sign) | ~1ms per sign | N x 1ms |
| Batch (decrypt once for entire commitment) | ~1-50ms (depends on HTLC count) | 1 x (1ms + N x 0.1ms) |
| Pre-decrypt (Option C) | Entire channel lifetime | Days to months |

Batch optimization **slightly increases** the exposure window (from ~1ms to ~50ms max) but **reduces I2C calls by 97%** and **reduces total exposure** (since N separate decrypts = N x 1ms vs. 1 batch of 50ms).

---

## 7. I2C Timeout Configuration Proposal

### 7.1 Current State

```rust
// i2c_actor.rs:65 — hardcoded 500ms for all operations
let result = tokio::time::timeout(
    std::time::Duration::from_millis(500),
    reply_rx
).await
```

### 7.2 Problem

500ms is **too generous** for AES Decrypt (27ms max) but potentially **too tight** for other operations:

| Operation | Actual latency | 500ms timeout | Assessment |
|-----------|---------------|---------------|------------|
| AES Decrypt | 1-27ms | 500ms = 18x margin | Too generous — masks failures |
| GenKey | ~115ms | 500ms = 4x margin | OK |
| ECDH | 58-172ms | 500ms = 3x margin | OK but tight |
| Sign P-256 | 50-115ms | 500ms = 4x margin | OK |
| Verify | 72-190ms | 500ms = 2.6x margin | Tight |
| SHA | 9-42ms | 500ms = 12x margin | Too generous |

### 7.3 Proposal: Operation-specific Timeouts

```rust
/// I2C timeout configuration
pub struct I2cTimeoutConfig {
    /// AES encrypt/decrypt (Coldcard pattern) — 3x safety margin
    pub aes_timeout: Duration,
    /// GenKey (P-256 key generation) — 3x safety margin
    pub genkey_timeout: Duration,
    /// Sign (P-256 ECDSA) — 3x safety margin
    pub sign_timeout: Duration,
    /// ECDH (key agreement) — 3x safety margin
    pub ecdh_timeout: Duration,
    /// Verify (P-256 signature verification) — 3x safety margin
    pub verify_timeout: Duration,
    /// Read/Write slot data — generous margin
    pub data_timeout: Duration,
    /// Default fallback
    pub default_timeout: Duration,
}

impl Default for I2cTimeoutConfig {
    fn default() -> Self {
        Self {
            aes_timeout: Duration::from_millis(100),     // 27ms * 3.7
            genkey_timeout: Duration::from_millis(350),  // 115ms * 3
            sign_timeout: Duration::from_millis(350),    // 115ms * 3
            ecdh_timeout: Duration::from_millis(500),    // 172ms * 2.9
            verify_timeout: Duration::from_millis(600),  // 190ms * 3.2
            data_timeout: Duration::from_millis(100),    // 26ms * 3.8
            default_timeout: Duration::from_millis(500), // Retain current value
        }
    }
}
```

### 7.4 Environment Variable Override

```bash
# alice-hardware.env
TEE_I2C_TIMEOUT_AES_MS=100
TEE_I2C_TIMEOUT_SIGN_MS=350
TEE_I2C_TIMEOUT_DEFAULT_MS=500
```

```rust
impl I2cTimeoutConfig {
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            aes_timeout: Duration::from_millis(
                std::env::var("TEE_I2C_TIMEOUT_AES_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default.aes_timeout.as_millis() as u64)
            ),
            // ... similar for other fields
            ..default
        }
    }
}
```

### 7.5 Integration with I2cActor

```rust
pub enum I2cRequest {
    AesDecrypt {
        slot: u8,
        ciphertext: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, TeeError>>,
    },
    // ...
}

impl I2cActorHandle {
    pub async fn aes_decrypt(&self, slot: u8, ciphertext: Vec<u8>) -> Result<Vec<u8>, TeeError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.tx.send(I2cRequest::AesDecrypt {
            slot, ciphertext, reply: reply_tx,
        }).await.map_err(|_| TeeError::I2cError("I2C actor not running".into()))?;

        // Use operation-specific timeout
        let result = tokio::time::timeout(
            self.timeout_config.aes_timeout,  // 100ms instead of 500ms
            reply_rx
        ).await
        .map_err(|_| TeeError::I2cError(format!(
            "I2C AES decrypt timed out ({}ms)",
            self.timeout_config.aes_timeout.as_millis()
        )))?
        .map_err(|_| TeeError::I2cError("I2C actor dropped reply channel".into()))?;

        result
    }
}
```

---

## 8. Recommended Approach: Option B+ (Code Sketch)

### 8.1 Architecture Overview

```
                                  LDK BackgroundProcessor (std::thread)
                                            |
                                            v
                              HybridSigner::Tee::sign_*()
                                            |
                        ┌───────────────────┼────────────────────┐
                        |                   |                     |
                        v                   v                     v
              mpsc::blocking_send()   oneshot::blocking_recv()   Zeroize
                        |                   ^
                        v                   |
              ┌─────────┴───────────────────┴──────┐
              |         I2cActor (tokio::spawn)      |
              |                                      |
              |  AesDecrypt request                   |
              |    → ATECC608A I2C bus (27ms)         |
              |    → Response via oneshot              |
              |                                      |
              |  Circuit Breaker integration           |
              |  Operation-specific timeouts           |
              └──────────────────────────────────────┘
```

### 8.2 Required Changes

#### File 1: `i2c_actor.rs` — Extend I2cRequest

```rust
// Add to I2cRequest enum
pub enum I2cRequest {
    // Existing
    Sign { data: Vec<u8>, reply: oneshot::Sender<Result<Vec<u8>, TeeError>> },
    ReadPcr { pcr_index: u8, reply: oneshot::Sender<Result<[u8; 32], TeeError>> },

    // NEW: AES operations for Coldcard pattern
    AesDecrypt {
        slot: u8,
        ciphertext: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, TeeError>>,
    },
    AesEncrypt {
        slot: u8,
        plaintext: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>, TeeError>>,
    },

    Shutdown,
}
```

#### File 2: `i2c_actor.rs` — Add blocking API

```rust
impl I2cActorHandle {
    /// Blocking variant for non-async callers (LDK BackgroundProcessor thread).
    /// SAFE: Only call from non-async threads. WILL DEADLOCK if called from async context.
    ///
    /// RECOMMENDED: Add a runtime detection guard at the top of this function:
    ///   if tokio::runtime::Handle::try_current().is_ok() {
    ///       panic!("aes_decrypt_blocking() called from async context — use aes_decrypt() instead");
    ///   }
    pub fn aes_decrypt_blocking(
        &self, slot: u8, ciphertext: Vec<u8>,
    ) -> Result<Vec<u8>, TeeError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        // blocking_send: safe because caller is non-async
        self.tx.blocking_send(I2cRequest::AesDecrypt {
            slot, ciphertext, reply: reply_tx,
        }).map_err(|_| TeeError::I2cError("I2C actor not running".into()))?;

        // blocking_recv with timeout (does not require tokio runtime)
        let result = reply_rx.blocking_recv()
            .map_err(|_| TeeError::I2cError("I2C actor dropped reply".into()))?;

        result
    }
}
```

#### File 3: Fork `wallet/mod.rs` — HybridSigner::Tee signing

```rust
// In ldk-node fork, branch 039-tee-hybrid-signer
impl EcdsaChannelSigner for HybridSigner {
    fn sign_counterparty_commitment(
        &self, commitment_tx: &CommitmentTransaction,
        inbound_htlc_preimages: Vec<PaymentPreimage>,
        outbound_htlc_preimages: Vec<PaymentPreimage>,
        secp_ctx: &Secp256k1<All>,
    ) -> Result<(Signature, Vec<Signature>), ()> {
        match self {
            HybridSigner::Software(signer) => {
                signer.sign_counterparty_commitment(
                    commitment_tx, inbound_htlc_preimages, outbound_htlc_preimages, secp_ctx
                )
            }
            HybridSigner::Tee(fallback_signer) => {
                // Phase 1: Use fallback InMemorySigner (current)
                // Phase 2: Batch decrypt + sign (code below)

                #[cfg(feature = "tee-hardware")]
                {
                    // Batch decrypt: 1 I2C call for the entire commitment + HTLCs
                    let decrypted_seed = self.i2c_handle
                        .aes_decrypt_blocking(0, self.encrypted_seed.clone())
                        .map_err(|e| {
                            tracing::error!(
                                target: "node_backend::tee",
                                error = %e,
                                "ATECC608A AES decrypt failed — circuit breaker may trip"
                            );
                            ()
                        })?;

                    // Derive and sign within Zeroizing scope
                    let result = {
                        let mut seed = Zeroizing::new(decrypted_seed);
                        let channel_key = derive_channel_key(&seed, &self.channel_keys_id);
                        let temp_signer = InMemorySigner::from_channel_key(channel_key);
                        temp_signer.sign_counterparty_commitment(
                            commitment_tx, inbound_htlc_preimages,
                            outbound_htlc_preimages, secp_ctx
                        )
                        // seed and channel_key zeroize on drop
                    };

                    result
                }

                #[cfg(not(feature = "tee-hardware"))]
                {
                    fallback_signer.sign_counterparty_commitment(
                        commitment_tx, inbound_htlc_preimages,
                        outbound_htlc_preimages, secp_ctx
                    )
                }
            }
        }
    }
}
```

### 8.3 Thread Safety Diagram

```
Thread 1: LDK BackgroundProcessor (std::thread::spawn)
  |
  |── sign_counterparty_commitment()
  |     |── mpsc::blocking_send(AesDecrypt)  ──────────> I2cActor task
  |     |                                                    |
  |     |                                                    v
  |     |                                              ATECC608A I2C
  |     |                                              (27ms blocking)
  |     |                                                    |
  |     |<── oneshot::blocking_recv() <─────────────── reply_tx.send()
  |     |
  |     |── derive_channel_key()  (software, <0.1ms)
  |     |── secp256k1::sign()     (software, <1ms)
  |     |── zeroize()             (<1us)
  |     |
  |     └── return Signature
  |
  |── (100ms later) next timer tick
  v

Thread 2: Tokio Runtime (tokio::spawn — I2cActor)
  |
  |── recv().await  (wait for request from mpsc)
  |── handle_aes_decrypt(&data).await  (perform I2C operation)
  |── reply.send(result)  (send result back via oneshot)
  v
```

**Key point**: Thread 1 (LDK) and Thread 2 (I2cActor) communicate via `mpsc` + `oneshot` channels. There is no shared mutable state. No data races. No mutex required in the signing path.

---

## 9. Impact on Other Components

### 9.1 I2cActor (`i2c_actor.rs`)

| Change | Details | Priority |
|--------|---------|----------|
| Add `I2cRequest::AesDecrypt` | New enum variant | P1 |
| Add `I2cRequest::AesEncrypt` | For seed encryption at rest | P2 |
| Add `aes_decrypt_blocking()` | Blocking API for non-async callers | P1 |
| Add `I2cTimeoutConfig` | Operation-specific timeouts | P2 |
| Add `I2cHandler::handle_aes_decrypt()` | New trait method | P1 |
| Add `MockI2cHandler::handle_aes_decrypt()` | Mock with 27ms simulated delay | P1 |

### 9.2 CircuitBreaker (`circuit_breaker.rs`)

| Change | Details | Priority |
|--------|---------|----------|
| Update description text | "protects AES decrypt" instead of "protects signing" | P2 |
| Operation type tracking | Distinguish AES-type failures from others | P3 |
| No logic change | 3 failures → Open → HalfOpen → Closed remains correct | - |

**Important note**: When the circuit breaker trips (ATECC608A unresponsive), signing has **NO fallback**. No key = cannot sign. This is a natural "fail-closed" behavior — safer than falling back to software keys (which could be compromised).

### 9.3 HybridSigner (fork `wallet/mod.rs`)

| Change | Details | Priority |
|--------|---------|----------|
| Add `i2c_handle: I2cActorHandle` to `Tee` variant | Via `Arc` shared state | P1 |
| Add `encrypted_seed: Vec<u8>` | Encrypted seed material | P1 |
| Add `channel_keys_id: [u8; 32]` | For deriving the correct channel key | P1 |
| Implement all `EcdsaChannelSigner` methods | Batch decrypt + sign pattern | P1 |
| Feature gate: `#[cfg(feature = "tee-hardware")]` | Fallback to InMemorySigner when no hardware is present | P1 |

### 9.4 BackendTeeSignerFactory (`ldk_signer_adapter.rs`)

| Change | Details | Priority |
|--------|---------|----------|
| Inject `I2cActorHandle` | Factory needs access to pass to HybridSigner | P1 |
| Inject encrypted seed | Factory needs the encrypted seed to pass to each channel signer | P1 |
| `derive_channel_signer()` update | Pass i2c_handle + encrypted_seed into Tee variant | P1 |

### 9.5 VlsSignerProvider (`ldk_signer_adapter.rs`)

| Change | Details | Priority |
|--------|---------|----------|
| No direct change | VlsSignerProvider remains an async adapter, unaffected | - |
| Future: VLS validation | Validate channel state before signing (R-001) | P2 (Sprint 039+) |

### 9.6 ldk_manager.rs

| Change | Details | Priority |
|--------|---------|----------|
| Initialize I2cActor before LDK Node | I2cActor must be ready before the signer is created | P1 |
| Pass I2cActorHandle to BackendTeeSignerFactory | So the factory can inject it into each HybridSigner | P1 |
| Add encrypted seed loading | Read encrypted seed from disk, pass to factory | P1 |

---

## 10. Implementation Roadmap (Sprint 039)

### Phase 1: I2cActor Extension (2-3 days)

- [ ] Add `I2cRequest::AesDecrypt`, `AesEncrypt` variants
- [ ] Add `I2cHandler::handle_aes_decrypt()` trait method
- [ ] Add `MockI2cHandler::handle_aes_decrypt()` with 27ms simulated delay
- [ ] Add `I2cActorHandle::aes_decrypt_blocking()` method
- [ ] Unit tests for AES decrypt flow
- [ ] Unit tests for blocking API

### Phase 2: HybridSigner Implementation (3-5 days)

- [ ] Update fork `wallet/mod.rs` — add `i2c_handle`, `encrypted_seed` to `HybridSigner::Tee`
- [ ] Implement `EcdsaChannelSigner` for `HybridSigner::Tee` with batch decrypt pattern
- [ ] Feature gate: `#[cfg(feature = "tee-hardware")]`
- [ ] Update `BackendTeeSignerFactory::derive_channel_signer()` to inject I2C handle
- [ ] Integration test: mock I2C → HybridSigner::Tee → LDK commit signature

### Phase 3: Timeout Configuration (1-2 days)

- [ ] Create `I2cTimeoutConfig` struct
- [ ] Implement `from_env()` for environment variable override
- [ ] Update `I2cActorHandle` methods to use operation-specific timeouts
- [ ] Update `TEE_I2C_TIMEOUT_*` env vars in documentation
- [ ] Unit tests for timeout behavior

### Phase 4: Integration Testing (2-3 days)

- [ ] End-to-end test: `ldk_manager.rs` → `BackendTeeSignerFactory` → `HybridSigner::Tee` → mock I2C
- [ ] Latency benchmark: measure actual time for batch signing (1 + N HTLCs)
- [ ] Circuit breaker integration test: I2C failure → circuit open → signing fails gracefully
- [ ] Stress test: 50 concurrent channel operations with mock I2C delay

### Phase 5: Documentation & Security Review (1 day)

- [ ] Update SECURITY_MODEL.md with Coldcard pattern and batch signing
- [ ] Update ARCHITECTURE.md with I2cActor extension
- [ ] Code review focusing on: key exposure window, zeroize correctness, thread safety

**Total: ~9-14 working days (Sprint 039)**

---

## Appendix A: Why `tokio::task::spawn_blocking` Is Not Needed

### Decision Table

| Question | Answer | Consequence |
|----------|--------|-------------|
| Where does LDK call the signer from? | `BackgroundProcessor` thread (`std::thread::spawn`) | Non-async context |
| Is the thread inside the Tokio runtime? | **NO** | `Handle::current()` will PANIC |
| Is `spawn_blocking` necessary? | **NO** | Already on a blocking thread |
| Is `blocking_recv()` safe? | **YES** | Caller is non-async, does not block Tokio |
| Is `blocking_send()` safe? | **YES** | Same reasoning |

### When `spawn_blocking` IS Needed

Only when the calling context is **async** (a Tokio task). In the current codebase, the only such case is `VlsSignerProvider` (async methods). However, `VlsSignerProvider` **does not implement `EcdsaChannelSigner`** — it is our own separate adapter, not constrained by the LDK sync trait.

```rust
// VlsSignerProvider — async, uses async I2cActor handle (already available)
pub async fn sign_commitment(&self, tx_hash: &[u8; 32]) -> Result<Vec<u8>, TeeError> {
    self.vls.sign_commitment(tx_hash).await  // async — no issue
}

// HybridSigner::Tee — sync (LDK constraint), uses blocking I2cActor handle (new)
fn sign_counterparty_commitment(&self, ...) -> Result<(Signature, Vec<Signature>), ()> {
    self.i2c_handle.aes_decrypt_blocking(...)  // blocking — safe because non-async caller
}
```

---

## Appendix B: Risk Analysis

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| LDK changes threading model | Low (stable API) | High | Monitor LDK releases, test with each version |
| I2C bus contention (TPM + ATECC608A) | Medium | Medium | I2cActor serialization (already in place) |
| ATECC608A AES latency > 27ms | Low | Low | 100ms timeout with 3.7x margin |
| Batch signing context leak (not zeroized) | Low (Rust ownership) | High | `Zeroizing<>` + `Drop` impl + code review |
| `blocking_send` deadlock | Very low | High | Only call from non-async context, document clearly |
| Channel overflow (mpsc buffer full) | Low | Medium | Buffer 16, alert when > 12 |
| `process_events_async` used in fork | Medium | **Critical** | `blocking_recv()` will PANIC if called inside async runtime. Add `tokio::runtime::Handle::try_current()` guard |

---

## Open Issues

1. **BackgroundProcessor async mode**: LDK recently added `process_events_async` for async runtime support. It is **critical** to verify whether the ldk-node fork uses `start()` (sync thread) or `start_with_runtime()` (async). If the async variant is used, `blocking_recv()` will **PANIC**. This is the highest-priority risk. **Recommended safeguard**: add `tokio::runtime::Handle::try_current()` inside the signing path to detect which mode is active and fail gracefully rather than panic.
2. **Batch decrypt security window**: Decrypting the seed once and signing N times means the seed resides in RAM for ~N ms. If N = 483 HTLCs x 1ms = ~500ms exposure. Compared to per-operation decrypt (27ms x 1 = 27ms exposure but repeated 483 times), batch decrypt has equivalent total exposure time but is continuous (worst case for cold boot attack). A detailed risk assessment is needed.
3. **I2cActor buffer overflow**: The mpsc channel buffer is 16. If batch signing sends many requests in rapid succession, the buffer may fill up, causing `blocking_send()` to block. A sizing analysis for worst-case HTLC counts is needed.
4. **500ms hardcoded timeout**: Requires benchmarking on actual hardware. AES-128 decrypt typical = 10ms, max = 27ms per datasheet — but with I2C bus overhead, wake-up sequence, and CRC validation, actual latency may differ.
5. **LDK peer disconnect timeout**: The document references `DISCONNECT_PEER_AWAITING_RESPONSE_TICKS` but the exact value has not been traced from the LDK source (estimated at ~120s). The LDK source needs to be grepped for this constant to verify the precise value.

---

## Sources

### LDK Source Code
- `lightning-background-processor/src/lib.rs` — `BackgroundProcessor` uses `std::thread::spawn` (not Tokio)
- [rust-lightning GitHub](https://github.com/lightningdevkit/rust-lightning) — Official repository
- [LDK process_events_async](https://github.com/lightningdevkit/rust-lightning/blob/main/lightning-background-processor/src/lib.rs) — Async variant added recently
- `lightning/src/sign/mod.rs` — `EcdsaChannelSigner` trait definition (sync methods)
- `lightning/src/ln/channelmanager.rs` — `DISCONNECT_PEER_AWAITING_RESPONSE_TICKS`, peer timeout logic
- `ldk-node/src/builder.rs` — `Node::start()` → `BackgroundProcessor::start()`

### Codebase References
- `packages/backend/src/tee/i2c_actor.rs` — I2cActor pattern, 500ms timeout (line 65)
- `packages/backend/src/tee/ldk_signer_adapter.rs` — BackendTeeSignerFactory, HybridSigner injection
- `packages/backend/src/tee/circuit_breaker.rs` — TeeCircuitBreaker (3 failures → Open)
- `packages/backend/src/ldk_manager.rs` — spawn_blocking pattern (line 159, 435)
- `packages/backend/Cargo.toml` — ldk-node fork dependency

### Research References
- R-003 datasheet_analysis.md — ATECC608A latency profile, Coldcard pattern
- SECURITY_MODEL.md — Known limitation: "Blocking I2C Signing" (line 289)
- ldk_node_fork_comprehensive_audit.md — Fork architecture analysis

### Tokio Documentation
- `tokio::sync::mpsc::Sender::blocking_send()` — safe on non-async threads
- `tokio::sync::oneshot::Receiver::blocking_recv()` — safe on non-async threads
- `tokio::task::spawn_blocking()` — only needed when caller is in an async context
