# Decentralized Confidential-Compute Mesh — PoC Extracts

A portfolio extract of work I did in 2026 on a decentralized "compute marketplace" project: a mesh of home-run nodes that rent each other VMs and AI inference, settle in Lightning sats, and use hardware-rooted Confidential Compute (CC) so the renter can verify the operator isn't reading their workload.

This repo is **not the full product** — it's a curated set of artefacts (code, infra, write-ups) from the slices I owned, published with the project owner's awareness as a record of what I built and what I learned.

---

## The product context (in one paragraph)

Imagine a mesh of small Linux boxes — Raspberry Pi 5s, Radxa boards, mini-x86 — each running the same OS. Any of them can be an "operator" (rent its CPU/RAM/GPU to others) or a "renter" (consume someone else's compute). Trades are paid per minute in Lightning sats. The novel part isn't the marketplace, it's the **trust shape**: before the renter sends data, the operator's box has to prove cryptographically — using its CPU's secure-enclave hardware — that the OS image being run is the audited one, no operator tampering possible. The renter's browser verifies this *itself*, against the chip vendor's root of trust. That's "real" CC, as opposed to "we promise we don't peek."

I contracted on this project for ~3 months and owned the CC integration end-to-end.

---

## What I shipped

### Phase A — Cloud SEV-SNP, real attestation, end-to-end demo

A live demo where Alice (renter, in her browser) rents a VM from Bob (operator, running in a Google Cloud N2D-Milan SEV-SNP machine), sees an attestation badge verify against the AMD root certificate chain locally in her browser, clicks Start, and within ~60s is staring at a Linux desktop streamed in over noVNC. Not a mock — Alice's backend pulls the raw SEV-SNP report + ARK/ASK/VCEK chain over L402, verifies the AMD ECDSA P-384 signatures with the `sev` Rust crate, and the badge only goes green if the chain is valid and the report's `report_data` matches the per-session nonce.

What's reproduced here:

- [`phase-a-cloud-cc/backend/sev_snp_verifier.rs`](phase-a-cloud-cc/backend/sev_snp_verifier.rs) — the verifier
- [`phase-a-cloud-cc/frontend/`](phase-a-cloud-cc/frontend/) — the React 19 panel + hook that renders the badge against a provider-polymorphic attestation DTO (so a future ARM/RISC-V provider drops in without UI rework)
- [`phase-a-cloud-cc/infra/cc-host/`](phase-a-cloud-cc/infra/cc-host/) — bootstrap script that pulls a fresh attestation report on the host + fetches AMD's KDS certs
- [`phase-a-cloud-cc/infra/cc-tenant/`](phase-a-cloud-cc/infra/cc-tenant/) — Packer image bake + spawn script that cut tenant cold-boot from 10–15 min to ~60s
- [`sprint-reports/2026-05-02-phase-a-shipped.md`](sprint-reports/2026-05-02-phase-a-shipped.md) — the end-of-phase report

### Phase C — Same flow, but on a $15 Radxa Zero 3W (the bring-up attempt)

This was the differentiator: prove the same chip-rooted attestation works on a small ARM SBC, not just on cloud hardware, so the marketplace can be genuinely decentralized rather than "cloud with extra steps." Target: Rockchip RK3566, OP-TEE on the TrustZone secure world, a custom Trusted Application doing the attestation.

I did not deliver this. What I delivered instead was a **clean empirical characterisation of why it doesn't work on this chip with vendor-shipped firmware**, hard enough that the next engineer can skip the dead-ends. Three independent vendor-controlled gates, confirmed individually:

1. **Upstream OP-TEE doesn't boot on RK3566.** The plat-rockchip port for this SoC doesn't exist in the upstream tree. I added a minimum-viable port ([`platform_rk3566.c`](phase-c-rk3566-attempt/plat-rockchip-rk3566/platform_rk3566.c) + [`platform_config` block](phase-c-rk3566-attempt/plat-rockchip-rk3566/platform_config_rk3566_block.h) + [`conf.mk` block](phase-c-rk3566-attempt/plat-rockchip-rk3566/conf_rk3566_block.mk)) and got it to build clean and flash correctly — but the CPU never reaches instruction 1 of `_start`. I confirmed this with a UART poke patched directly into the very first instruction of `entry_a64.S`: silent. The BL31→BL32 handoff dies before any C code runs. Community ports for this SoC family don't exist either.
2. **The vendor's shipped BL32 binary refuses all our TA signing keys.** I pivoted to working with the firmware as-shipped — wrote a real attestation TA ([`node_attestation_ta.c`](phase-c-rk3566-attempt/node-attestation-ta/ta/node_attestation_ta.c): persistent RSA-2048 keypair via `TEE_STORAGE_PRIVATE`, RSASSA-PSS-SHA256 sign on a nonce+measurement) and a libteec smoke-test client ([`smoke_test.c`](phase-c-rk3566-attempt/node-attestation-ta/host/smoke_test.c)). Both built and signed cleanly. Vendor BL32 rejected the TA with `0xffff000f origin=3` (`TEEC_ERROR_TARGET_DEAD`) using two different candidate signing keys (upstream master + OP-TEE 3.13 default; MD5-distinct, so I know I tried different things). The vendor's signing key is closed.
3. **The vendor's BL32 has no built-in attestation primitives we could call instead.** I probed 10 candidate Pseudo-TA UUIDs (attestation, sealed-key storage, fTPM, secure storage, vendor-specific Widevine/OEM_CRYPTO) — only `PTA_SYSTEM` responds. Everything attestation-related is stripped from the build. Linux's `keyctl` TEE-backed trusted-keys subsystem is also not in the kernel image.

The reusable artefact from Phase C is the playbook in [`docs/op-tee-rk3566-bring-up-playbook.md`](docs/op-tee-rk3566-bring-up-playbook.md) — written as a template ("here's how you bring CC up on any SoC the mesh later adopts"), not as a board-specific cookbook. It documents the hardware variables you have to pin down for any new chip, the firmware substrate you need, and the questions to answer before you spend a sprint on it.

End-of-phase write-up: [`sprint-reports/2026-05-03-phase-c-walls.md`](sprint-reports/2026-05-03-phase-c-walls.md).

### Adjacent: marketplace work the week before

[`sprint-reports/2026-04-26-marketplace.md`](sprint-reports/2026-04-26-marketplace.md) — the sprint before Phase A, covering the rental-flow UI, polymorphic attestation DTO design, and the first R-012 firmware substrate validation on Radxa. Included here for continuity.

---

## What's not here

- The full product codebase (mesh primitives, gossip protocols, capability router, L402 micropayment middleware, builtin apps, AI agent system, the broader marketplace) — that belongs to the project owner and isn't mine to redistribute. Everything in this repo is code paths I authored, infra I wrote, or write-ups I produced.
- Live demo coordinates (public IPs, peer pubkeys) are redacted in the sprint reports. The demo flow is real, but the specific deployment is the owner's to keep or take down.
- The full R-012 PoC report (the playbook here is the deliverable extracted from it).

---

## Tech surface (so you can scan the stack)

- **Rust** (Axum 0.8, Tokio 1.38, Diesel 2.1, custom LDK fork for Lightning) — backend, the verifier crate, the OP-TEE host-side smoke-test
- **C / OP-TEE 4.x** — the Trusted Application (GlobalPlatform TEE Internal API: `TEE_GenerateKey`, persistent objects, RSASSA-PSS), the plat-rockchip port
- **React 19 + TypeScript 5.8 + Tailwind 4** — provider-polymorphic attestation panel
- **GCP / Packer / gcloud** — Phase A infra
- **AMD SEV-SNP** — ARK/ASK/VCEK chain verification, snpguest, KDS
- **ARM TrustZone / TF-A / u-boot mainline (FIT, binman)** — Phase C boot-chain bring-up
- **Lightning** — the marketplace settlement layer (touched at the protocol boundary; not extracted here)

---

## Layout

```
.
├── README.md                       — this file
├── docs/
│   └── op-tee-rk3566-bring-up-playbook.md
├── phase-a-cloud-cc/
│   ├── backend/sev_snp_verifier.rs
│   ├── frontend/
│   └── infra/
│       ├── cc-host/bootstrap.sh
│       └── cc-tenant/{packer/, spawn.sh, startup-script.sh, README.md}
├── phase-c-rk3566-attempt/
│   ├── plat-rockchip-rk3566/       — OP-TEE OS port (excerpts)
│   └── node-attestation-ta/        — TA + libteec smoke-test client
└── sprint-reports/
    ├── 2026-04-26-marketplace.md
    ├── 2026-05-02-phase-a-shipped.md
    └── 2026-05-03-phase-c-walls.md
```

---

## License & credit

The OP-TEE port files and the TA are BSD-2-Clause (matching the upstream OP-TEE project, which I was extending). The Rust verifier and React components are MIT for the parts I authored. Sprint reports and the playbook are CC-BY-4.0.

Written by Tan Nguyen Huu &lt;nguyenhuutan262004@gmail.com&gt;, 2026.
