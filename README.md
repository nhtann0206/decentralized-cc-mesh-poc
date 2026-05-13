# Decentralized Confidential-Compute Mesh — PoC Extracts

A portfolio extract from work I did in 2026 on a decentralized "compute marketplace": a mesh of home-run nodes that rent each other VMs and AI inference, settle in Lightning sats, and use hardware-rooted Confidential Compute (CC) so the renter can cryptographically verify the operator isn't reading their workload.

This repo is **not the full product** — it's a curated set of the slices I owned end-to-end, published as a record of what I built.

---

## The product, in one paragraph

Imagine a mesh of small Linux boxes — Raspberry Pi 5s, Radxa boards, mini-x86 — each running the same OS. Any of them can be an *operator* (rent its CPU/RAM/GPU to others) or a *renter* (consume someone else's compute). Trades are paid per minute in Lightning sats. The novel part isn't the marketplace, it's the **trust shape**: before the renter sends data, the operator's box has to prove cryptographically — using its CPU's secure-enclave hardware — that the OS image being run is the audited one, no operator tampering possible. The renter's browser verifies this *itself*, against the chip vendor's root of trust. That's "real" CC, as opposed to "we promise we don't peek." I owned the CC integration end-to-end across the contract.

---

## What's inside

Each folder is a distinct slice of the product. Read the folder's README for the full story; the one-liners below are the elevator pitch.

### [`tee-foundation/`](tee-foundation/)
**The substrate every later CC piece plugged into.** A polymorphic trait (`HardwareTrust`) for hardware-rooted attestation, mock implementations for CI/dev, and the HTTP surface the rental flow calls when it needs a fresh attestation report. The trait shape is what kept the marketplace UI/backend identical whether the operator's hardware was AMD SEV-SNP, ARM TrustZone, or a future RISC-V target.

### [`hybrid-signer/`](hybrid-signer/)
**The Lightning channel keys live in hardware, not process memory.** A custom signer for the Lightning Development Kit (LDK) that holds the channel-private seed in a secure element or TEE, drops cleanly into LDK's `NodeBuilder` without forking it, bridges the sync↔async impedance mismatch between LDK's signer API and async I2C transport, and fails closed for big-money HTLCs while staying useful for tiny routing fees if the hardware path degrades. The most security-sensitive piece of the contract and the one I'm proudest of.

### [`slm-edge-inference/`](slm-edge-inference/)
**Small Language Model inference on the node itself, as a metered service.** Operators set a memory budget; the module picks which models fit, manages their lifecycle (load on demand, evict under pressure, drain in-flight requests before swap), exposes them on the API as just-another-provider so the agent framework calls them the same way it calls a cloud LLM. Lets a home node sell inference for sats without phoning home to OpenAI.

### [`cross-node-rental/`](cross-node-rental/)
**The marketplace mechanics — how one node rents a VM from another, billed per minute over Lightning.** KVM/libvirt provider with idempotent pause/resume, a billing state machine that survives crashes and reconciles cleanly, BOLT12-based per-minute settlement (the renter's timer, the operator's verification, the events that tie them to the LDK builtin app), cross-node L402 authentication so an unknown renter is auto-onboarded on first request, and the peer-workspace UI that puts a face on all of it.

### [`novnc-streaming/`](novnc-streaming/)
**Click Start, see your rented Linux desktop in the browser tab within a minute.** A WebSocket proxy that forwards binary RFB frames from a tenant VM's VNC server out to a noVNC client in the renter's browser, plus the desktop-image bake script that makes the tenant boot fast enough for "one-click" to actually feel one-click. This is the change that turned the product from "IaaS for technical people" into "rent a computer like you rent a parking spot."

### [`phase-a-cloud-cc/`](phase-a-cloud-cc/)
**Real Confidential Compute on the rental flow, demoable end-to-end.** Renter rents a VM hosted on an AMD SEV-SNP machine; renter's browser pulls the raw attestation report plus the AMD certificate chain, verifies the AMD ECDSA signatures *locally*, only goes green if the chain validates and the report binds to the per-session nonce. Then noVNC streams the desktop in. No mock, no "trust me" — verifiable.

### [`phase-c-rk3566-attempt/`](phase-c-rk3566-attempt/)
**The same flow attempted on a $15 ARM SBC — and why it doesn't work on this chip.** The differentiator for the decentralized story is that operators run on small home hardware, not just cloud. I attempted to bring the same chip-rooted attestation up on a Radxa Zero 3W (Rockchip RK3566). It does not work, for three independent vendor-controlled reasons that I characterised empirically and documented as a reusable playbook for any future hardware target.

### [`docs/`](docs/) and [`sprint-reports/`](sprint-reports/)
The OP-TEE bring-up playbook (written as a template for any future SoC, not a board-specific cookbook), and three end-of-sprint reports written in the voice they were originally delivered in.

---

## Tech surface

Rust (Axum, Tokio, Diesel ORM, custom LDK fork) for the backend; C for the OP-TEE Trusted Application and the plat-rockchip port; React 19 + TypeScript for the renter-facing UI; GCP + Packer + libvirt for the infrastructure layer; AMD SEV-SNP, ARM TrustZone, Lightning Network (BOLT12) as the protocol substrates touched.

---

## What's not here

The full product codebase — mesh primitives, gossip, capability router, L402 micropayment middleware, the broader agent system, the rest of the marketplace — belongs to the project owner and isn't mine to redistribute. Everything in this repo is code I authored or co-authored, infrastructure I wrote, or write-ups I produced. Live demo coordinates (IPs, peer pubkeys) are redacted.

Written by Tan Nguyen Huu &lt;nguyenhuutan262004@gmail.com&gt;, 2026.
