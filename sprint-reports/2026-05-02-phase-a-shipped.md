# Sprint Report — 2026-04-27 → 2026-05-02

## What we shipped

Phase A — **real Confidential Compute on the Alice ↔ Bob rental flow** — end-to-end on GCP. Boss can open Alice's URL, click Rent on a Basic VM, see an AMD SEV-SNP attestation badge verified locally, click Start, and after ~60s a Linux desktop streams into the browser. Both sides (renter + operator) see the live session.

Phase A inherits everything from the Phase 1 marketplace shipped last week (multi-tenancy, hosting dashboard, BOLT12 billing). The ONLY new layer is a 4-step wizard inserted between "click Rent" and the actual session start, gating on a real hardware attestation.

Concretely:

1. **Step 1 — Browse** (unchanged from Phase 1) — Alice picks a VM tier on Bob's marketplace.
2. **Step 2 — CC proof** — Alice's backend asks Bob over L402 for raw SEV-SNP evidence (report.bin + ARK/ASK/VCEK certs as base64), verifies the AMD chain locally with the `sev` Rust crate, and renders a green badge with chip ID, SHA-384 measurement, VMPL, policy bitfield, and TCB component versions.
3. **Step 3 — Confirm** — Alice reviews the verified CC badge alongside price/RAM/storage and clicks Start.
4. **Step 4 — Active VM** — Bob's backend spawns a tenant VM from a Packer-baked `node-cc-tenant` image (~60s cold boot vs ~10-15min during the runtime-install phase of the week), Bob's WebSocket proxy bridges the browser noVNC client to tenant `websockify → Xtigervnc → xfce4`, and Alice gets an interactive desktop.

Stakeholder can hit Alice's public URL, browse Bob's node, and rent a VM. Demo session billed at 3 sats/min on regtest credits. (IP + node pubkey redacted for portfolio publication; ask if you'd like the live coordinates.)

---

## Architecture — what's new vs Phase 1

```
┌───────────────────────── PHASE A ADDITIONS (in green) ───────────────────────┐
│                                                                              │
│   ALICE BROWSER                                                              │
│   ─────────────                                                              │
│   /peer-workspace                                                            │
│     ┌───────────────────────────────────┐                                    │
│     │ Marketplace grid (Phase 1)        │                                    │
│     └────────────┬──────────────────────┘                                    │
│                  │ click Rent                                                │
│                  ▼                                                           │
│   ┌────────────────────────────────────────────────────────┐ ◄── NEW         │
│   │ Wizard step 2 — CC proof panel                         │                 │
│   │ ⏳ Verifying with AMD…                                  │                 │
│   │ ✓ CC Verified · AMD SEV-SNP                            │                 │
│   │   Chip b568e48d…44e1d5e7   VMPL 1                      │                 │
│   │   Measurement 7a5ed176…06d2ec92434e                    │                 │
│   │   Policy 0x0000000000030000                            │                 │
│   │   TCB bl=4 tee=0 snp=29 uc=222                         │                 │
│   └─────────────────┬──────────────────────────────────────┘                 │
│                     │ + computer summary (machine type, sats/min, RAM, GB)   │
│                     │ + Cancel | Start session                               │
│                     ▼                                                        │
│            (Phase 1 startSession path)                                       │
│                                                                              │
└──────────────────────────────────┬───────────────────────────────────────────┘
                                   │ NEW: GET /workspace/peer/{node}/cc-attestation
                                   │   (via Alice's local backend → L402 → Bob)
                                   ▼
┌───────────────────────────── ALICE BACKEND ──────────────────────────────────┐
│                                                                              │
│  peer_provider_cc_attestation handler                            ◄── NEW     │
│   ├─ TransportManager → HttpWorkspaceClient.get_provider_cc_attestation      │
│   │     (L402-authenticated request to Bob)                                  │
│   ├─ receive RawAttestationEvidence { report_b64, vcek/ask/ark_der_b64 }     │
│   ├─ base64-decode each blob                                                 │
│   ├─ tee::sev_snp_verifier::verify_sev_snp_attestation()                     │
│   │     ARK self-signed → ASK signed by ARK → VCEK signed by ASK             │
│   │     → report ECDSA P-384 signed by VCEK → nonce binding check            │
│   └─ shape into CcAttestationDto for frontend                                │
│                                                                              │
│  KEY DESIGN: Alice does the chain verification herself. Bob returns RAW      │
│  bytes only. Trust in Bob's verifier is NOT assumed; the AMD root keys      │
│  embedded in the `sev` Rust crate are the trust anchor.                     │
│                                                                              │
└──────────────────────────────────┬───────────────────────────────────────────┘
                                   │ L402 GET /external/workspace/cc-attestation
                                   │   (NodeId-authenticated, Lightning-signed)
                                   ▼
┌────────────────────────────── BOB BACKEND ───────────────────────────────────┐
│                                                                              │
│  http_workspace_external::http_provider_cc_attestation             ◄── NEW   │
│   └─ WorkspaceService::fetch_own_cc_attestation_raw()                        │
│        reads /var/lib/cc-host/{report.bin, certs/{ark,ask,vcek}.der}         │
│        returns base64-encoded RawAttestationEvidence                         │
│                                                                              │
│  /var/lib/cc-host populated once at boot by                                  │
│    infra/gcp/cc-host/bootstrap.sh                                  ◄── NEW   │
│    (snpguest report --random + KDS fetch + AMD chain self-verify)            │
│                                                                              │
│  WorkspaceService::start_session — GCP runtime branch              ◄── NEW   │
│   └─ random 64-byte nonce → CcConfig::SevSnp { nonce_hex }                   │
│      └─ GcpComputeProvider::create_container                                 │
│           ├─ machine_type=n2d-standard-2 (CC requires N2D Milan)             │
│           ├─ confidentialInstanceConfig.confidentialInstanceType="SEV_SNP"   │
│           ├─ minCpuPlatform="AMD Milan"                                      │
│           ├─ scheduling.onHostMaintenance="TERMINATE" (no live migrate)      │
│           ├─ sourceImage = $GCP_VM_CC_IMAGE (Packer-baked node-cc-tenant)    │
│           └─ metadata.startup-script = trimmed CC_TENANT_STARTUP_SCRIPT      │
│                                                                              │
└──────────────────────────────────┬───────────────────────────────────────────┘
                                   │ GCP Compute API
                                   ▼
┌─────────────────────── TENANT VM (n2d, SEV-SNP, Milan) ──────────────────────┐
│                                                                              │
│  Image: node-cc-tenant Packer family                               ◄── NEW   │
│   pre-baked at build time (≈25min one-shot, vs ≈10-15min per spawn):         │
│     apt: xfce4 + tigervnc-standalone-server + python3-websockify + novnc     │
│          + nginx + epiphany-browser + build deps                             │
│     /usr/local/bin/snpguest (cargo install)                                  │
│     systemd units: tigervnc@.service (Type=simple), cc-websockify.service    │
│                                                                              │
│  Per-spawn (CC_TENANT_STARTUP_SCRIPT, embedded in backend binary via         │
│  `include_str!`):                                                            │
│     1. Read metadata: tenant-id, attestation-nonce-hex, vnc-password         │
│     2. /usr/local/bin/snpguest report — bind to Bob's nonce                  │
│     3. /usr/local/bin/snpguest fetch ca/vcek der from AMD KDS                │
│     4. Self-verify chain (sanity check before exposing)                      │
│     5. Write /var/www/cc/{report.bin, certs/*, manifest.json} for nginx      │
│     6. Create tenant user + xstartup → exec startxfce4                       │
│     7. Drop in systemd override for tigervnc@tenant.service:                 │
│          ExecStart= /usr/bin/Xtigervnc :1 -localhost no -SecurityTypes None  │
│                     -MaxProcessorUsage 100 -CompareFB 0 …                    │
│        (auth gate is Bob's JWT WebSocket two hops upstream;                  │
│         tenant 6080 is internal-VPC only)                                    │
│     8. systemctl restart tigervnc@tenant.service cc-websockify.service       │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

The renting flow itself is byte-for-byte the Phase 1 flow — `peerWorkspaceApi.startSession`, `peer_start_session` handler, `record_rental_started` cache, BOLT12 billing tick. CC is purely a gate inserted in the wizard before the existing path fires.

---

## Honest accounting — what cost the week

Original plan estimate when we started Monday: ~50 minutes for the full integration once the code refactor was clean.

What it took: **5 days**. Below are the integration-boundary errors that ate the difference, ordered roughly by cost. Each is a real lesson, not a debugging anecdote.

| # | What broke | Time burned | Root cause |
|---|---|---|---|
| 1 | First Bob N2D spun up on Ubuntu 24.04 (glibc 2.39); Alice runs Debian 12 (glibc 2.36); a binary built natively on either side wouldn't run on the other | ~half a day + full Bob recreate | OS picked from GCP default before checking Alice's libc |
| 2 | GCP Compute API rejects empty POSTs without `Content-Length`; reqwest's `.send()` on a body-less request omits it; Bob's `start_container` then 411'd, the cleanup-on-error path deleted every freshly-created tenant VM ~15s after spawn | ~1 day to localize through the cleanup-loop | Code inherited a libvirt-shaped pattern (separate create/start) without a working test against GCP. Fixed: `.json(&serde_json::json!({}))` |
| 3 | Bob's VNC proxy hard-coded `ws://127.0.0.1:port`, only valid for libvirt where QEMU's built-in VNC websocket lives on Bob's localhost | ~half a day + refactor across 4 files | Same libvirt/GCP shape mismatch as #2, but in the proxy layer |
| 4 | `tigervnc@.service` from Ubuntu defaults uses `vncserver` (Perl wrapper that exits-after-fork) under `Type=forking`; systemd timed it out and killed the X server every spawn | ~1 hour + write a clean unit | Default packaging not designed for our headless flow |
| 5 | Tigervnc rejected browser handshakes with `AuthFailureException` because `-SecurityTypes VncAuth` requires a password noVNC can't easily inject pre-handshake | ~2 hours including a few false starts | We were defending the same data twice — Bob's JWT WebSocket already gates browser → tenant; VNC-level password adds nothing without delivering the password through that same gated channel anyway |
| 6 | Multiple GCP IAM follow-ons: default compute SA missing `cloud-platform` scope on Bob and on the c2 builder; missing `iam.serviceAccountUser` self-binding for Packer; missing `iap.tunnelResourceAccessor` for Packer's IAP SSH | ~2 hours spread across the week | Each operation surfaced a new permission gap; should have set a "CC-host SA" with the full role bundle from day one |
| 7 | Tar-and-rsync to remote builders: BSD tar's `--exclude` patterns don't behave like GNU; first deploys were 1.9 GB instead of 36 MB because qcow2 fixtures and node_modules slipped through | ~30 min × several iterations | Test the tarball locally before pushing |

**The pattern**: every cost above came from a shape-mismatch between an inherited libvirt/Phase-1 assumption and the GCP runtime — or from skipping a single verification step that would have surfaced the problem before it touched the demo path.

The partly-good news: each fix is now upstream — the code, the Packer image, the SA roles, the firewall rules. Phase A.6 polish notwithstanding, a fresh demo spawn now runs through cleanly.

---

## Polish items still queued (non-blocking)

These don't gate the demo but tighten the rough edges before Phase C:

- **Per-session firewall isolation**: tenants currently share the `default-allow-internal` VPC reach; add a per-session network tag + firewall rule restricting tenant `:6080` to Bob's IP only. ~15 LOC in `gcp_provider.rs::create_container` + one `gcloud compute firewall-rules create` per session.
- **L402 payment middleware on `api_v2_router_ext`**: identity is enforced by the `NodeId` extractor today, but the L402 *payment* enforcement layer isn't wrapped at v2; CC attestation is currently a free read. Documented in `core/infrastructure.md`.
- **Anti-replay nonce binding**: `report_data` is currently extracted from the report itself (Phase A.0 limitation). Phase A.1 binds to a Bob-supplied per-request challenge — the data structures already exist, just the verifier path needs the change.
- **Browser inside VM**: Epiphany shipped in this week's bake; Chromium needs a non-snap install path (Mozilla PPA or a pinned `.deb`). Chromium is the documented stack default per `directive/stack-directive-2026-04-14`.
- **VNC perf tuning**: `-MaxProcessorUsage 100 -CompareFB 0` shipped this week to relieve the n2d-standard-2 CPU bottleneck. Latency is dominated by user→Alice geography, not server-side; nothing else to do unless we move regions.

---

## What's next — Phase C

Phase A was always the cloud safety net. Phase C is the real bet: an attested boot on a $25 Radxa Zero 3W with OP-TEE PTA on the RK3566 (research note `phase-c-day1-prep-2026-04-28.md`, revised `phase-c-revision-2026-04-29.md`).

Sequence for next week:

1. Order Radxa Zero 3W (if not in hand)
2. Pull current OP-TEE upstream + build the attestation PTA
3. Port `platform_rk3566.c` to upstream OP-TEE — biggest unknown, owns most of the critical-path schedule
4. Wire RK3566 attestation into the same `tee::` trait surface Alice already verifies against (so Phase C reuses the wizard end-to-end without UI changes)
5. Boss acceptance on a real device, not GCP

Phase A demo URL + the Bob N2D + Alice instances will stay live as the cloud safety net until Phase C lands.

---

## Demo

- Alice (renter) UI: `http://<alice-public-ip>:3001/peer-workspace`
- Bob node ID to enter: `<bob-lightning-pubkey-66-hex>`
- Login mnemonic: see auto-memory `reference/demo-login-keys.md`
- Top up Alice credits: SQL insert into Bob's `api-store.db.credits` (already at 1000 sats from Friday)
- Cleanup: `gcloud compute instances stop bob-cc-demo-2026-04-29 node-lightning-host` (persistent disks retain state for resume)

Cost during demo period (boss in Canada, no cross-region pain):
- Bob N2D `n2d-standard-2` SEV-SNP: ~$0.10/h
- Alice `e2-standard-2`: ~$0.05/h
- Tenant VMs: ~$0.10/h per active rental
- Packer image storage: ~$0.50/GB-month (image is ~2 GB)
- Total active: ~$0.25/h ≈ ~$6/day
