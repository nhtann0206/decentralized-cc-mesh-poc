# Sprint Report — 2026-05-03

## What we shipped

The Phase A demo — **real Confidential Compute on the Alice ↔ Bob rental flow on GCP** — remains live and demoable on demand. Open Alice's URL, click Rent on a Basic VM, see the AMD SEV-SNP attestation badge verify, click Start, and after ~60s a Linux desktop streams into the browser. One command (`gcloud compute instances start ...`) brings the demo back up; another stops it for cost. Runbook saved at `sprints/research-hub/runbook/phase-a-cc-test-runbook.md`.

The next step — **same rental flow but with the operator (Bob) running on a small home-node device instead of a cloud datacenter** — was the focus this week. The home-node target is a Radxa Zero 3W single-board computer (Rockchip RK3566 chip, ~$15, the kind of small device a customer would plug in at home). The technical goal: have that small device produce the same kind of hardware-rooted attestation that the cloud Bob produces today, so Alice's Verify-CC step works identically against either kind of operator.

We could not deliver that this week. The blocker is vendor firmware on the chip, not engineering effort. The full evidence is below; the short version is:

- The chip's secure-world firmware is shipped by the vendor as a sealed binary that only loads applications signed with the vendor's private key (which they don't publish) and exposes none of the standard attestation primitives we'd otherwise build on top of.
- The open-source equivalent firmware, which we'd happily run instead, has no working port for this specific chip.

Both paths to delivering hardware-rooted attestation on this device end at vendor-controlled gates. We confirmed both ends empirically before reporting; the rest of this report is the evidence.

---

## What we tried, what it told us

### Architecture — where the gate is

```
                    ALICE BROWSER
                         │
                         │  click Rent  ►  Verify CC  ►  Confirm  ►  VM
                         │
                         ▼
                    ALICE BACKEND
                         │
                         │  asks the operator for a CC attestation
                         ▼
                    OPERATOR (Bob)
        ┌────────────────┴─────────────────┐
        │                                  │
   ┌────────────┐                  ┌──────────────┐
   │ GCP cloud  │                  │ Radxa device │
   │ (Phase A)  │                  │ (this week)  │
   └─────┬──────┘                  └──────┬───────┘
         │                                │
   AMD SEV-SNP                  ┌─────────┴─────────┐
   chip-rooted                  │ secure-world FW   │ ◄── vendor-locked
   attestation                  │ on the chip       │     gate is here
   (works today)                └─────────┬─────────┘
                                          │
                            chip-rooted attestation
                            (this week's target)
```

The Alice browser, Alice backend, and the rental UI flow are unchanged across the two operators. Only the bottom-right box — what the home-node operator can produce — is in question.

### Path 1 — open-source secure-world firmware on the chip

We built the open-source equivalent of the vendor's secure-world firmware for this chip family from scratch (the chip-specific port doesn't exist in the upstream tree), wrote our attestation logic on top, packaged everything into a flashable image, and booted the device. The image is byte-for-byte verified flashed; the chip's CPU does not execute a single instruction of our firmware after the bootloader hands off to it. It is silent at the very first step. We confirmed this with a serial-console probe placed before any C code runs, before memory-management setup, before anything: nothing.

This matches a finding our team had already documented in April: porting the open-source firmware to this specific chip is research-grade work without public precedent. The community of developers working on this SoC family has not produced a working port either, despite the chip being two years old.

### Path 2 — vendor-shipped secure-world firmware + our own application

We pivoted to working with the firmware as it ships. The chip boots fine with the vendor binary; we wrote our attestation as a regular Trusted Application that the firmware loads at runtime. The application compiles cleanly and signs correctly with the standard developer key. The vendor firmware refuses to load it — every Trusted Application must carry a signature made by the vendor's private RSA key, and the vendor does not publish that key. We tested with two distinct candidate signing keys (the current upstream open-source default and the one shipped with the firmware version on this chip — different keys, verified by hash). Both rejected with the same TEE-side error code.

### Path 3 — vendor-shipped firmware, no app loading needed

If the vendor had simply enabled the standard upstream attestation primitive in their firmware build, the chip would expose it directly to user-space and we wouldn't need to load anything. We probed the firmware for ten candidate built-in primitives covering attestation, sealed-key storage, TPM-style quoting, secure storage, and several vendor-specific options. Only the most basic system primitive responds; everything attestation-related is stripped from the build. We additionally verified the kernel-side fallback (Linux's `keyctl` trusted-keys subsystem with TEE backing) is not built into the kernel image either.

### What was not pursued, and why

| Approach | Why we didn't pursue it |
|---|---|
| Continue Path 1 indefinitely | High uncertainty, no precedent, would consume the next several batches before we even know if it's possible |
| Hex-patch the vendor firmware binary to swap in our public key | Risky, fragile, a software bypass of a vendor security mechanism rather than a clean engineering path |
| Add a third-party crypto chip to the device | We do not have such a chip in hand. The chip hat physically attached to our device is a different ESP32-based companion board that doesn't include one |
| Use the companion ESP32 board's own crypto features | Would require coordination with the team member who built that board (HAT pinout, firmware status), creating a person-dependency we shouldn't take on without scoping |
| Software TPM emulation / measured-boot-only attestation | Not hardware-rooted; equivalent to mock for the trust model. The whole point of this work is to deliver something stronger than that |

---

## Vision impact

Multi-VM self-host on operator-owned hardware (home-node tenancy + chip-rooted CC verification) is the product flow per the 2026-04-14 stakeholder reconfirmation, not a long-term aspiration. The Phase A demo proves the customer flow end-to-end with real chip-rooted attestation today, on cloud hardware; the strategic intent of running the same flow on a small home-node operator is the differentiator and is unchanged. What's blocked this week is *this specific chip* being able to be that home-node operator. The customer journey, wallet, marketplace UI, billing primitives — none of those are touched by this week's blocker. Phase A continues to show the rental flow.

What this week did pay for, even without a finished home-node demo:

- **A polymorphic attestation interface** in the codebase. The frontend and backend now treat any chip-rooted attestation provider behind one interface — whether that provider is AMD SEV-SNP (today), a different chip we adopt later, or a separate hardware chip wired to the device. When the home-node story unblocks (whichever direction), the wiring on top is already done.
- **A clear, evidence-backed answer** to the question "is this chip the right home-node target?" — backed by hard probing rather than guesswork. The next decision can be made on data instead of assumption.

---

## Decision needed

The home-node hardware-rooted CC story is the differentiator of the decentralized-mesh product (per `directive/multi-agent-vm-vision.md` — multi-VM self-host on operator-owned hardware is THE product flow). Direction for the next batch:

**(a) Stay on this chip, treat the home-node CC as a multi-batch research effort.** Keep working on the open-source firmware port; accept high uncertainty and that no demoable home-node CC ships in the meantime.

**(b) Add a third-party hardware crypto chip to the existing Radxa device** (small parts cost; ATECC608A or Infineon SLB9670 TPM 2.0 are the standard candidates). The Radxa stays as the home-node target; the new chip provides the chip-rooted identity the vendor firmware doesn't expose. Real hardware-rooted attestation, predictable integration scope.

**(c) Pivot the home-node target SoC** — either an older Rockchip chip where the open-source firmware does work, or the RISC-V option (per the earlier 2026-04-23 direction). Different hardware order; longer ramp before the next CC demo attempt.

Recommendation: **(b)**. Smallest-cost, predictable integration, keeps the home-node hardware identity intact, fits in a single batch.

### Tactical asks while strategic direction is being decided

Two exploratory tracks runnable in parallel with current resources, no parts purchase or strategic decision dependency:

1. **PoC security path using the ESP32-C5 already on the existing companion HAT.** Not equivalent to a dedicated secure element — it cannot deliver the same attestation chain — but a real chip with hardware crypto features available now. The PoC surfaces the upper bound of "what we can do with current hardware" and informs whether (b) parts-purchase is strictly needed or whether the existing ESP32 covers a meaningful slice.
2. **Confirm the rest of the home-node rental flow on the Radxa — the operator-spawns-tenant-VMs-on-its-own-hardware part — without the CC step yet.** Phase A's operator runs GCP-managed VMs; the Radxa hosting tenant VMs on its own resources (Firecracker / KVM-style local hypervisor) is unvalidated. De-risking it now means when the CC gate unlocks, both halves of the home-node story are ready to wire together.

---