# Sprint Report — 2026-04-20 → 2026-04-26

## What we're building

Last week we shipped the cross-node VM rental flow on GCP: Alice (renter) paying Bob (operator) per-minute in Bitcoin Lightning to run a real KVM virtual machine, with the guest desktop streamed into the browser. This week we widened the loop on **both sides** and put it in boss's hands.

Two new surfaces landed:

1. **Multi-tenancy on the renter side** — a user can hold several active rentals across multiple peers at once, each with its own live billing and its own desktop session, surviving browser refresh.
2. **Hosting dashboard on the provider side** — the same node in its other role, showing who is currently renting capacity, with matched revenue numbers.

Together these are the mesh in two lenses. The Phase 1 ask was "show the rental flow works end-to-end." What we handed boss this Friday is closer to "here's the full marketplace, from both seats, live and interactive."

We also booted the Node OS on Radxa Zero 3W 1 GB ARM hardware — the home-node form-factor — and exposed the UI on LAN. That was the first move of the Phase 2 work.

---

## Marketplace two-way view — architecture diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          ALICE'S NODE (Renter)                          │
│                                                                         │
│   Browser                                                               │
│   ──────                                                                │
│   /my-rentals  ──────┐                                                  │
│     (cards × N)      │  GET /api/v2/workspace/my-rentals                │
│                      │     (read LOCAL cache, no peer round-trip)       │
│                      ▼                                                  │
│                  ┌─────────────────────────────┐                        │
│                  │  Alice backend              │                        │
│                  │                             │                        │
│                  │  my_rentals cache table     │                        │
│                  │  ├─ session_id              │                        │
│                  │  ├─ peer_node_id            │                        │
│                  │  ├─ last_seen_status        │                        │
│                  │  ├─ last_seen_cost_sats     │                        │
│                  │  └─ bolt12_offer            │                        │
│                  │                             │                        │
│                  │  cron "my-rentals-refresh"  │──┐                     │
│                  │  every 15s                  │  │                     │
│                  └─────────────────────────────┘  │                     │
│                                                   │                     │
│   /my-rentals/:session_id                         │                     │
│     ┌─────────────┐                               │                     │
│     │ VNC canvas  │◄──── ws /api/v2/workspace/    │                     │
│     │ (react-vnc) │      peer/{node}/sessions/    │                     │
│     └─────────────┘      {id}/vnc?token=<jwt>     │                     │
│                         (inline JWT validation,   │                     │
│                          no auth_middleware —     │                     │
│                          browser WebSocket can't  │                     │
│                          set Authorization hdr)   │                     │
└───────────────────────────────────────────────────┼─────────────────────┘
                                                    │
                          L402-authenticated HTTP (per-peer, 15s)         
                          + Lightning BOLT12 payment (per-minute)         
                          + WebSocket VNC proxy                           
                                                    │                     
┌───────────────────────────────────────────────────┼─────────────────────┐
│                          BOB'S NODE (Operator)    │                     │
│                                                   │                     │
│                  ┌─────────────────────────────┐  │                     │
│                  │  Bob backend                │◄─┘                     │
│                  │                             │                        │
│                  │  workspace_sessions  ───────┤                        │
│                  │  workspace_billing_ledger   │                        │
│                  │  (source of truth)          │                        │
│                  │                             │                        │
│                  │  ldk-node app (retry loop   │                        │
│                  │   around node.start() —     │                        │
│                  │   5 attempts, exp backoff — │                        │
│                  │   caught real esplora flake │                        │
│                  │   "after 2 attempts")       │                        │
│                  │                             │                        │
│                  │  libvirt provider           │                        │
│                  │  ├─ virt-install            │                        │
│                  │  ├─ --input tablet,bus=usb  │                        │
│                  │  │  (NEW — absolute cursor) │                        │
│                  │  ├─ qcow2 overlay per VM    │                        │
│                  │  └─ QEMU VNC + websocket    │                        │
│                  └─────────────┬───────────────┘                        │
│                                │                                        │
│   Browser                      │                                        │
│   ──────                       │                                        │
│   /hosting ◄───────────────────┘  GET /api/v2/workspace/hosting/*       │
│    ├─ 4 summary tiles (active, tenants, revenue 24h, revenue lifetime)  │
│    ├─ "You're selling" catalog (list_local_computers)                   │
│    └─ Active tenants table (renter pubkey · computer · status · revenue)│
│                                                                         │
│   Tenant VM — Debian 13, full kernel (cloud kernel purged from base),   │
│   XFCE + Chromium, streams VNC to Alice's browser.                      │
└─────────────────────────────────────────────────────────────────────────┘

  ┌──────────────────────────────────────────────────────────────────────┐
  │                        ALICE-on-RADXA (Phase 2 trial, 2026-04-22)    │
  │                                                                      │
  │   Radxa Zero 3W 1 GB · RK3566 aarch64 · Debian 12 · WiFi             │
  │                                                                      │
  │   Same Node OS binary (cross-compiled aarch64 zigbuild on Bob GCP).  │
  │   Same BIP39 as Alice-GCP → same user pubkey 048aad04... but LDK     │
  │   node_id is random per-install (architectural gap, see below).      │
  │                                                                      │
  │   Boot + UI via LAN verified. 603 MB RAM free. LDK retry loop fired  │
  │   on first boot ("after 2 attempts") — same fix, ARM binary.         │
  │                                                                      │
  │   NOT yet wired: rent-from-Radxa flow (blocked on identity           │
  │   consolidation), CC attestation (hat inventory pending).            │
  └──────────────────────────────────────────────────────────────────────┘
```

---

## What was delivered

### 1. Multi-tenancy UX — `/my-rentals` page + backend cache

A user can now rent N VMs across M peers and see all of them in one place. Previously the UI tracked one active session in React state; rent a second one and the first fell off. The fix is a proper local cache on the renter's node:

- **Migration** `2026-04-21-000001_create_my_rentals` — `my_rentals(session_id, peer_node_id, user_id, status, cost_sats, elapsed_seconds, price_sats_per_min, bolt12_offer, …)` keyed by session_id, indexed on (user_id, status) and (status, last_poll_at).
- **Cron** `*/15 * * * * *` invokes a refresh task that polls each non-terminal row's provider via `http_workspace_client.get_session` and writes back status / cost / elapsed / price. Failures keep last-known state rather than flipping status — transient peer unreachability doesn't corrupt the UI.
- **Endpoint** `GET /api/v2/workspace/my-rentals` reads the cache directly. The frontend polls localhost every 5 s; no peer round-trip on render.
- **Card component** with click-through to a detail page that embeds the real VNC desktop (`react-vnc`), plus per-card Stop.
- **Toasts** on `startSession` / `stopSession` mutation errors — fixes the silent-fail seen during testing (where "Insufficient credits" came back as 422 and the button just blinked).

### 2. Hosting dashboard — `/hosting` page (the other half of the marketplace)

The renter-only framing was always going to be half the story. The hosting view is the operator-side mirror:

- **Backend** `GET /workspace/hosting/{sessions,summary,offerings}`. `HostedSession` DTO = `WorkspaceSession` fields plus `renter_node_id` (the renter's Lightning pubkey). `HostingSummary` gives `active_sessions`, `distinct_tenants`, `revenue_last_24h_sats`, `lifetime_revenue_sats` (summed from `workspace_billing_ledger` TickCharge deltas).
- **Frontend page**: 4 summary tiles + "You're selling" catalog (what the node offers) + Active tenants table with renter pubkey, status chip, elapsed, revenue. No new components invented — all existing Card/Badge/PageLayout primitives.
- **Privacy fix along the way**: `list_running_sessions()` returned every user's sessions on the host. The L402 external endpoint now uses `list_running_sessions_for_user(user_id)` — a caller can only see their own sessions on a remote node.
- **Dead code deleted**: `/my-machines` route + `MyMachinesPage.tsx` — the cross-IA audit confirmed it was legacy single-session scaffold, superseded by `/my-rentals`. Removed in the same PR to avoid drift.
- **Sidebar**: new "Hosting" entry; old "My Machines" removed.

Same session on both sides: Alice's displayed cost and Bob's displayed revenue match within about 5 s (the polling interval), and stopping on one side propagates to the other within that same window.

### 3. Input + display fixes (M7a)

- **Mouse alignment** inside the guest was wrong after mousedown events. Initial fix was adding `--input tablet,bus=usb` to `virt-install` (USB tablet = absolute pointer vs PS/2 = relative). But clicks *still* landed wrong. Root cause was deeper: the base image was booting the **Debian cloud kernel** (`linux-image-*-cloud-amd64`), which ships without `xhci_hcd` and `usbhid`. The guest kernel never saw the tablet. Fix was to purge the cloud kernel from the base image (`virt-customize -a base-debian-13.qcow2 --run-command "apt-get remove --purge -y linux-image-6.12.74+deb13+1-cloud-amd64 && update-grub"`) so GRUB boots the full kernel on next rental. Verified by user.
- **Rate display** on rental cards was `0 sats/min`. `peer_start_session` inserts with `price_sats_per_min=0` (the renter doesn't know the rate before Bob confirms), and the cache refresh never overwrote it. Threaded `session.price_sats_per_min` through `update_rental_from_peer` — UI now shows the correct rate on the next refresh tick.
- **VncScreen focusOnClick** so keyboard input reaches the guest after clicking the canvas.

### 4. LDK init retry loop — **a real root-cause fix, not a bandaid**

Every second cold-start of Alice was leaving the `ldk-node` app in the `error` state, and with it every capability it provides (`core.lightning.sign_message`, `list_peers`, L402 auth signing). The UI would sit on "Loading peer marketplace…" forever. We had been patching around it (restart the backend, pray) and user correctly flagged that as a bandaid.

Root cause: `node.start()` synchronously fetches fee-rate estimates from the configured esplora (`blockstream.info/testnet`), and treats any timeout as fatal. Blockstream latency is variable, so roughly half of cold boots landed during a slow window → init returns -1 → app enters error state → capabilities never register. Same binary, same .so; the failure was environmental.

The fix lives in `packages/builtin-apps/ldk-node/src/node_manager.rs` — wrap `node.start()` in a 5-attempt exponential backoff (1 s, 2 s, 4 s, 8 s, 16 s) and log the attempt count on success so the retry path is observable. The third cold-boot after deploy proved it in production: Bob's LDK came up with `LDK node start attempt 1/5 failed (Updating fee rate estimates timed out.); retrying in 1s` followed by `LDK node started successfully after 2 attempts`. Without the fix that node would have been stuck.

Principled because (a) it addresses the actual failure mode — transient external dependency — not a symptom, (b) it doesn't swallow permanent outages (after 5 attempts the node exits with a clear error), (c) the retry count surfaces in logs so we can see whether esplora is getting flakier over time.

### 5. Radxa Zero 3W boot — first ARM home-node step

The product shape per boss's 2026-04-14 realignment is the mesh of operator-owned home boxes, not the GCP bootstrap. We pulled the Radxa Zero 3W 1 GB (RK3566 Cortex-A55 aarch64, Debian 12) forward into this sprint:

- **Build**: after a failed attempt with Mac Docker Desktop (nightly `-Z build-std` cross-compile was burning ~45 min on Apple Silicon with the laptop unusable), we moved the build onto Bob GCP where `cargo zigbuild --target aarch64-unknown-linux-gnu.2.36` runs in ~8 min. Two cross-compile snags along the way: `libudev-sys` pkg-config failure (fixed by `serialport = { version = "4.5", default-features = false }` in backend/Cargo.toml) and `node-app-graph-engine` duplicate `_node_app_entry` symbol when building the full workspace (graph-engine imports graph-core-tasks and both export cdylib entry — resolved by building targets individually rather than `--workspace`).
- **Deploy**: single tar with binary + 12 builtin .so + manifests + static frontend dist → scp to Radxa → extract to `~/node/`. No deb packaging — the Makefile's deb pipeline is designed for the Docker path and this was quicker.
- **Env + run**: minimal `node.env` (testnet, mock TEE, mock workspace runtime, fresh random signer seed), `nohup bin/lightning_node_backend --env node.env &`.
- **Verified**: boot clean, LDK retry loop fired once on cold start ("after 2 attempts"), RAM 603 MB free of 973 MB, HTTP `/api/v2/health` 200, LAN laptop at `http://192.168.3.22:3001` sees the onboarding page.
- **Onboarded**: same Alice BIP39 phrase, same user pubkey `048aad04…` in sidebar as Alice-GCP. Confirms client-side BIP39 → user identity is deterministic across devices.

The `sprints/research-hub/runbook/radxa-board-ops.md` captures the full deploy + troubleshooting steps so it doesn't have to live in shell history.

### 6. Boss self-test handoff

Boss asked for a self-run runbook, not a video demo. Shipped `sprints/research-hub/runbook/phase1-boss-self-test.md`: URLs, BIP39 seeds for Alice and Bob, Bob's node ID for the paste step, five scenarios walked top-to-bottom (rent twice → see in My Rentals → interact with VM desktop → see from Bob's Hosting view → stop one → hard-refresh to prove rehydration). Honest "deferred" section listing what Phase 1 explicitly didn't cover (TLS, discovery UX, real CC Badge wiring, credits manually pre-seeded). State readied in advance: Alice topped up to 500 000 sats on Bob, orphan VMs cleaned, stale rental rows cleared. Boss is testing now; we stay off GCP while that window is open.

### 7. OP-TEE PoC on Radxa Zero 3W — first hardware-side Confidential Compute proof

While boss was self-testing the GCP demo over the weekend, we executed R-012 in parallel on the Radxa Zero 3W. The goal per boss directive (2026-04-25) was narrow: prove ARM TrustZone with OP-TEE actually runs on the existing hardware as a PoC — not full Confidential Compute, just a "verify the substrate works" exercise.

**Outcome**: full BL31 → BL32 → kernel chain operational, TZASC firewall enforced, `/dev/tee0` userspace device live, and a minimal C ioctl test program proves Normal-World ↔ Secure-World SMC round-trip works in microseconds. Persistent config achieved (patched DTB + locked extlinux.conf) so the board auto-boots into OP-TEE-enabled state without UART intervention.

**Evidence captured** (full logs in deployment guide):

```
NOTICE: BL31: v2.3-896, fwver: v1.45
INFO:   Using opteed sec cpu_context!
I/TC:   OP-TEE version: 3.13.0-891-g9f2aca7d1, fwver: v2.15
I/TC:   Primary CPU initializing
I/TC:   Primary CPU switching to normal world boot

[ 13.024641] optee: probing for conduit method.
[ 13.024675] optee: revision 3.13 (9f2aca7d)
[ 13.025469] optee: dynamic shared memory is enabled
[ 13.025725] optee: initialized driver

$ sudo /tmp/tee_version
impl_id=1 impl_caps=0x1 gen_caps=0xd
  TEE_GEN_CAP_GP          YES
  TEE_GEN_CAP_PRIVILEGED  no
  TEE_GEN_CAP_REG_MEM     YES
  TEE_GEN_CAP_MEMREF_NULL YES
=== Round-trip OK: ioctl -> kernel -> SMC -> BL32 -> back ===
```

**Three critical gotchas surfaced** during 6 iterations of trial-and-error:

1. **`ttyFIQ0` console hijacking** — Rockchip Debian default cmdline conflicts with OP-TEE FIQ interception. When OP-TEE/BL32 is active, the `opteed` SPD claims non-secure FIQ for Secure World handling — the same FIQ line the Rockchip-specific `ttyFIQ0` console driver uses. Symptom looks like a kernel hang because UART output dies silently after `Starting kernel ...`. Fix: switch to `console=ttyS2,1500000n8` (standard 8250 driver, regular IRQ). Pattern is generic — any CC stack that uses async interrupts to enter the secure context will conflict with kernel drivers sharing that mechanism.

2. **TZASC firewall + missing `reserved-memory`** — Rockchip downstream `rk2410-nocsf` kernel DTB lacks both `reserved-memory/optee@8400000` and `firmware/optee` nodes. Kernel maps the OP-TEE region as ordinary RAM; slab allocator eventually hands out a page from inside `0x08400000–0x0A400000`; first store hits TZASC and raises a fatal SError with ESR_EL1 `0xbe000011` (EC=0x2F SError, IDS=1 vendor-specific firewall fault). This is also a generic pattern: memory firewalls require the kernel to opt out, not opt in. Fix: patch DTB to declare both nodes (`linaro,optee-tz` compatible + `method = "smc"`).

3. **Upstream `tee-supplicant` cannot attach to Rockchip BL32 v2.15** — BL32 reports `gen_caps = 0x0d` (PRIVILEGED bit `0x2` is missing). Hypothesis: Rockchip ships a closed BSP supplicant via private ABI and masks the upstream PRIVILEGED capability. Phase 2 blocker for any TA needing reverse RPC (filesystem, secure storage, time). Three resolution paths documented (path C: rebuild OP-TEE OS from source for RK3566 — recommended for fully-open NodeOS firmware claim).

**Generic CC bring-up checklist** distilled from the case study and applied to future hardware (Milk-V Jupiter 2 RISC-V Keystone, Pi 5, Apple Silicon, Intel TDX) — see deployment guide §V.1 for the 8-step framework.

**Deliverable**: [`docs/conf-com/research/tee/optee_rk3566.md`](../../../docs/conf-com/research/tee/optee_rk3566.md) — comprehensive deployment guide structured as **generic CC bring-up playbook** (Parts I, II, V) + **RK3566 case study with full evidence** (Part III + Appendices A-B raw UART logs + kernel panic stack + DTB diffs) + **adaptation sketches** for other targets (Part V.4). Method designed to transfer — RK3566 is the substrate for *learning the bring-up process*, not the production target. Long-term aim per stakeholder direction is Milk-V Jupiter 2 (RISC-V).

**R-012 status**: PARTIAL CLOSE. Task 1 (firmware build chain) ✅ done. Tasks 2–6 unblocked for Phase 2 implementation pending stakeholder decision on (a) supplicant resolution path and (b) RK3566-vs-Jupiter 2 hardware bet.

---

## Why these decisions

### Why HTTP + L402 polling for private session state, not gossip

The first instinct was to push session state over gossip — "it's mesh, so gossip." That was wrong. Gossip in the LDK fork is plain JSON broadcast to the whole LN network; pushing `"Alice is renting session X on Bob"` over it is a privacy regression. Centralized systems are at least 1-to-1. The right Phase 1 primitive for **private** bilateral state is what the rest of the codebase already uses for this class of data (`peer_slm`, `conversation`, `workspace_endpoint`): L402-authenticated HTTP poll, 1-to-1 between the two peers that actually need to see it. Lightning itself does the same for channel state (BOLT #2 bilateral, not gossip). Gossip stays reserved for **public** discovery (what a node offers) — that's M7b scope and intentionally deferred.

### Why retry loop in the ldk-node native app, not systemd restart

Three options for the LDK flaky-init problem. Retry in the native app — localized, preserves backend process continuity, successful retries are silent-observable via log. Systemd `Restart=on-failure` — valid but whole-backend restart for a single app's init failure feels heavy, and loses other app state. Catch-and-treat-as-non-fatal — unsafe, starting LDK without a fresh fee rate could affect channel decisions later. Retry wins on all three dimensions.

### Why delete `/my-machines` in the same PR as adding `/hosting`

Both touch the same mental space — "what I am doing with workspaces on this node." The audit confirmed `/my-machines` was a legacy single-session view that `/my-rentals` supersedes. Letting it sit around means the sidebar grows by one item boss-side and the user gets stuck on a stale page. Cleanup inside the feature PR is lighter than a separate cleanup PR that'll stall.

### Why build on Bob, not Mac or on the Radxa itself

Mac Docker cross-compile hit nightly `build-std` on Apple Silicon, 45+ min with the laptop unusable — user flagged it was "cắm hết tài nguyên, lag điên." Building natively on Radxa would take longer still (1 GB RAM, rustc swaps into oblivion). Bob is a c2-standard-8 with 32 GB already set up for Rust builds; cargo-zigbuild targeting `aarch64-unknown-linux-gnu.2.36` takes ~8 minutes and doesn't touch the user's local machine. Tech lead Vu uses the same pattern for Pi Zero prod binaries. Keeping it uniform matters when the deploy pipeline needs to be reproducible.

### Why we went into research mode when boss started testing

User's call, on the right principle. Touching GCP while boss is interacting with the runbook risks breaking his session mid-evaluation, and the week's worth of work is in his hands for review anyway. Better to use the window for depth: re-read every boss directive end-to-end, run the past week's implementation against them, and come out with an architectural debt register and a concrete Phase 2 sprint shape that survives pressure-testing. That document is `sprints/research-hub/research/phase2-readiness.md`, summarized below.

---

## Architectural debt register (from this week's research)

Fifteen items surfaced. Grouped:

**Keystone (unblocks multiple downstream items)**:
- **TEE `wrap_key` FFI stub** (`backend/src/tee/wrap_key.rs`, `// TODO Phase 2: implement via libteec FFI`). Blocks signer-seed encryption at rest, which blocks operator-blind session data, which blocks credible backup mesh, which blocks "tenant VM state survives host power-off" (the explicit Phase 2 crash-recovery ask). Fixing this one cascades.

**Identity** (design decision needed from boss before coding):
- **BIP39 ↔ LDK signer seed split**. Current: client-side BIP39 for user auth, node-local 64-byte random entropy for LDK — two independent roots of trust. This is a defensible design but contradicts boss's single-BIP39 identity vision. Three options laid out in the research doc (`§1.10`): strict BIP39 → BIP32 → LDK, strict split with separate LDK backup, hybrid. Needs boss call.

**Mass-market UX violations (conflict with 2026-03-19 directive)**:
- User still pastes a 66-char Lightning pubkey in `/peer-workspace`. The fix is the gossip `service_offering` schema (M7b) — infrastructure is there, schema is missing.
- CC Badge component exists from Sprint 039 scaffolding but is not wired into the workspace UI. Phase 1 honesty mandate says it should display "Development Mode" while the real attestation lands. Easy win.
- Raw terms in UI: `node_id`, `session_id`, `sats/min`. 2026-03-19 forbids crypto/protocol jargon outside DID.

**Phase 2 explicit asks, not yet started**:
- Crash recovery + state persistence. BACKUP_ARCHITECTURE.md was rewritten and approved 2026-04-14 (mesh-first, Reed-Solomon, Lightning-paid peer storage), but implementation is zero lines and blocked by the keystone TEE item.
- Hardware reproduction is partial — Radxa boot done, ATECC608A HAT integration pending inventory clarification (see below).
- Base agent VM template — OpenClaw v007 Packer image exists, but the question "is the Phase 2 template OpenClaw specifically, any runtime, or a user-choice marketplace?" needs boss.

**Hardware inventory surprise (flagged today)**:
Our design docs assume an ATECC608A + TPM on the security hat. The physical hat Tùng built has only an ESP32-C5-WROOM-1, and the ESP32 gatekeeper role was explicitly dropped 2026-02-25. The Phase 2 CC substrate currently has no hardware anchor on the board we have. Three paths to resolve: procure an ATECC608A breakout and wire to Radxa I2C, go software-only / RK3566 TrustZone-only for attestation, or clarify with Tùng/Vu whether a populated-chip variant of the hat exists. We have not picked; handled as research item next week.

**Silent ones worth naming**:
- Hardcoded default ports `3000` / `9735` scattered across ip_pool, social_network, rathole — defaults that only make sense in the GCP bootstrap, break on a mesh of home nodes with per-node port choices.
- Terminal session replay migrations are parked (5 files with `TODO: Uncomment when migrations are created`).
- Conversational UI has a hardcoded `"default_user"` fallback — its own test flags it.
- Billing interval = 60 s hardcoded in `workspace/service.rs`.
- Rathole NAT proxy exists in `src/rathole/` but has no L402 payment integration and no `proxy_registration` table. Infrastructure without economics — a node can run the Rathole server but can't charge for it.
- Autopilot (Lightning Grid core: channel management, rebalance, top-up) — zero lines.
- Spend-wallet $20–$200 constraint from Lightning Grid Vision — not enforced. Alice sits on 500 000 sats of pre-seeded credit on Bob, 10× the ceiling boss described.

---

## What the user can do today vs. what remains

What works end-to-end on the handed-off GCP stack:
- Log in as Alice with a BIP39 seed, rent 2+ VMs from Bob, see both live in "My Rentals," interact with the desktop in-browser, stop either independently, survive a browser refresh.
- Log in as Bob separately, see both tenants on "Hosting" with matching revenue and the offered VM tier catalog.

What works on Radxa today:
- Boot NodeOS, load the UI from a LAN laptop, onboard the same BIP39 → same user pubkey `048aad04…` as Alice-GCP.

What doesn't work yet:
- Rent from Radxa to GCP. The Alice-Radxa Lightning node_id is a fresh random `03669e…` because we haven't consolidated BIP39 → LDK seed derivation yet, so Bob treats her as a brand-new user with zero credits.
- Real CC attestation end-to-end. Mock path is wired; real ATECC608A signing is blocked on hardware inventory + TEE wrap_key FFI.
- Peer-replicated backup of tenant VM state. Design approved since 2026-04-14, no code.
- Home-node-behind-NAT (Rathole proxy marketplace with Lightning payment). Infrastructure exists, economics don't.

---

## What's next

Boss replied 2026-04-26 morning: *"the basic setup works. Good job!"* — Phase 1 demo passes. The same email opened a parallel track: a team-wide UI design contest for the Node website redesign. Presentation 2026-05-06 at 10 PM, $200 prize for one developer; submission is mandatory regardless of prize for the part of the surface each developer owns. Boss attached his own Cloud UI mockup as a starting point and recommends developing UI separately from code (Claude / Claude Design + iterative prompts). Our scope = the Cloud surface (rent + host + agent integration).

**Two parallel tracks for the next two weeks**:

1. **Cloud UI design** (mandatory, immediate priority) — synthesize boss's mockup (single-page Cloud landing, agent-first framing, earth-tone visual, gold/cream + Instrument Serif italic) with our prior depth in `sprints/research-hub/design/node-app (2).jsx` (provisioning state, backup/restore flows, error handling, empty states, multi-page coverage) into a comprehensive product spec for Cloud release. Output: detailed design doc (spec + flow diagrams + state tables) **plus** an interactive mock-data demo that boss can poke at directly. Standards to honor throughout: decentralized-first, agents-first, Bitcoin micropayments (sats per minute), Confidential Compute safety surfaced visibly. Deadline 2026-05-06.

2. **Phase 2 R-012 follow-up** — five open questions from the research doc still need boss's call before we pick up tools, plus two new ones from the OP-TEE PoC (supplicant resolution path; RK3566-vs-Jupiter 2 hardware bet). Reduced cadence (~30%) while the UI track runs.

The Phase 2 sprint sequence (2.1–2.5) outlined last week stands:

- **2.1 — Keystone + quick wins**: TEE `wrap_key` FFI (with tech lead Vu), gossip `service_offering` schema, CC Badge wiring ("Development Mode" honest until attestation lands), Pause/Resume UI + countdown timer, hardcoded user_id fix.
- **2.2 — Hardware + encryption**: session state encryption using 2.1 output, Radxa hat I2C wiring (once inventory confirmed), real CC attestation path, host-local crash recovery.
- **2.3 — Backup mesh (or defer)**: iroh-blobs + Reed-Solomon + libp2p per the approved architecture. Big scope; likely a go/no-go decision after 2.1.
- **2.4 — Networking + economics**: Rathole payment integration (L402), home-node-behind-NAT story, spend-wallet $20–$200 UX.
- **2.5 — Lightning Grid alignment (or Phase 3)**: autopilot channel management, mesh topology enforcement. Probably Phase 3 territory.

Shifted right by ~1 week to make room for the UI track.

Open questions for boss before tools come back out:

1. BIP39-vs-LDK seed design (A/B/C from research doc).
2. Agent VM template scope (OpenClaw specifically / any runtime / user-choice marketplace).
3. Fail-closed circuit breaker shipped/not-shipped contradiction.
4. Sprint sequence sign-off (2.1–2.5).
5. Backup mesh scope decision.
6. **NEW** — OP-TEE supplicant resolution path: patched upstream supplicant, Rockchip BSP blob, or rebuild OP-TEE OS from source for RK3566?
7. **NEW** — Continue RK3566 Phase 2 firmware customization (6+ weeks) or pivot Phase 2 effort to Milk-V Jupiter 2 procurement + Keystone bring-up?

Next immediate actions (in order): sprint report sent (this), Cloud UI brainstorm + spec docs scaffold, wait for boss decisions on the seven questions above, resume Phase 2 once the UI spec has a stable checkpoint.

---

## Honest notes on process

- User had to call out twice this week that code-first was the wrong reflex — once when I was about to add a "Hosting" page without reading the existing IA (led to the `/my-machines` deletion + the additive design, which was cleaner), and once when boss entered the testing window and I was still proposing code changes that could break his session. The correction in both cases was to read and think before typing.
- The LDK retry-loop fix started as a bandaid proposal and became a real fix only after user pushed back on "restart the service and hope." The fact that the fix *then caught a production flake on its next cold start* was the validation.
- The hardware-inventory mismatch (design assumed ATECC608A, reality has only ESP32-C5) was something I would have missed if we'd just started coding Phase 2 from the research doc. Worth sitting with the physical board + memory for another pass before any hardware-attached work starts.
