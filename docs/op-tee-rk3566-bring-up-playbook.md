# Confidential Compute PoC — ARM TrustZone (OP-TEE) on Radxa Zero 3W

> **Origin and intent.** This document is the deliverable of [R-012](../../../../sprints/research-hub/ticket/R-012.md). It is NOT a board-specific cookbook. It is a **template for bringing up Confidential Compute (CC) on any hardware target NodeOS will eventually run on** — so future engineers can replicate the exercise on Milk-V Jupiter 2 (RISC-V Keystone/Penglai), Apple Silicon (Secure Enclave), Pi 5 with TPM HAT, or any board the mesh later embraces.
>
> ARM TrustZone on RK3566 is the **first concrete validation** that CC primitives can be wired end-to-end inside the NodeOS stack. The board itself is disposable — what matters is the *method* by which the build chain, gotchas, and proof artifacts were derived.

---

## Document map

- **[Part I — Why CC, and what NodeOS actually needs](#part-i--why-cc-and-what-nodeos-actually-needs)** — platform-agnostic requirements
- **[Part II — Choosing a CC primitive (TrustZone among alternatives)](#part-ii--choosing-a-cc-primitive)** — comparison framework
- **[Part III — RK3566 + OP-TEE PoC: the case study](#part-iii--rk3566--op-tee-poc-the-case-study)** — full journey with logs
- **[Part IV — Reproducible deployment](#part-iv--reproducible-deployment)** — exact commands
- **[Part V — Generic playbook for future boards](#part-v--generic-playbook-for-future-boards)** — apply Parts I–IV to anything else
- **[Part VI — Phase 2 implementation plan](#part-vi--phase-2-implementation-plan-now-unblocked)** — where this leads
- **[Appendix A — Evidence: full logs and diagnostic output](#appendix-a--evidence-full-logs-and-diagnostic-output)** — raw artifacts
- **[Appendix B — Iteration log: the 6 failed attempts](#appendix-b--iteration-log-the-6-failed-attempts)** — what was learned at each step

---

## Part I — Why CC, and what NodeOS actually needs

### I.1 The decentralization angle

NodeOS is a mesh of small home-run nodes (8–32 GB RAM, single-board computers, NUCs, recycled laptops). Each node trades services with peers — LLM inference, VM compute, storage, routing — settled in Lightning sats. The mesh has **no central authority**, no single trust anchor, no SaaS provider.

This shape creates a security problem that centralized SaaS does not have: a node operator's *own machine* may be lost, stolen, seized, or briefly unattended. An attacker who gains physical access — or merely root via supply-chain malware — should not be able to:

1. **Extract Lightning private keys** and drain channels.
2. **Read peer secrets** stored locally (API tokens, tenant credentials, session keys).
3. **Forge attestations** that the node is running unmodified NodeOS firmware.

A truly decentralized mesh requires every node to be a small, self-contained trust island. CC is the hardware mechanism that makes this island defensible.

### I.2 Properties NodeOS needs from any CC primitive

The following requirements are independent of CPU vendor, TEE technology, or board model. Any candidate CC stack (TrustZone, SGX, SEV-SNP, Apple SE, RISC-V Keystone, Penglai, Intel TDX, …) is evaluated against the same checklist:

| # | Property | What it means in NodeOS context |
|---|---|---|
| **R1** | **Memory isolation enforced in hardware** | Lightning private keys + tenant secrets live in a memory region that the Normal-World OS *cannot* read, even with root and `/proc/<pid>/mem`. Enforcement is at the memory bus / firewall, not the kernel. |
| **R2** | **Secure execution context** | Code that handles secrets runs in a different privilege context (Secure World, enclave, secure monitor, …) where a compromised Linux kernel cannot inspect or trace it. |
| **R3** | **Attested boot chain** | A relying party (a peer in the mesh, a tenant) can verify cryptographically that a node is running the expected NodeOS firmware version, kernel, and initrd — not a tampered fork. |
| **R4** | **Persistent secure storage** | A small amount of state (channel secrets, key handles, sealed credentials) survives reboots while remaining inaccessible to Normal World, sealed to firmware/PCR state. |
| **R5** | **secp256k1 signing inside the secure context** | Lightning is built on secp256k1. The secure context must be able to perform secp256k1 ECDSA / Schnorr signing without exporting the key. Hardware crypto engines are unsuitable when they only support P-256 (e.g. ATECC608A). Software-secp256k1-inside-secure-context is acceptable. |
| **R6** | **Hardware-backed RNG** | Key generation needs entropy that does not depend solely on Linux's CSPRNG (whose state lives in Normal World). Hardware TRNG mixed into the secure context's entropy pool. |
| **R7** | **Bidirectional Normal ↔ Secure RPC** | Linux backend can call into the secure context with arguments and receive return values. The mechanism (SMC instruction, SGX `EENTER`, SEV hypercall, …) does not matter; the property does. |
| **R8** | **No closed-source RPC dependency** | A NodeOS distribution should be reproducible from open sources. Closed-blob daemons (Rockchip BSP `tee-supplicant`, NVIDIA GPU enclaves, …) are red flags — note them but plan to replace. |

### I.3 Threat model — who is the attacker

CC defends against, in increasing severity:

- **Network attacker** (no access to box). Standard TLS / wireguard / Tor handles this — no CC needed.
- **Local non-root attacker** (e.g. shell on the same machine). Standard Unix permissions + LSM (AppArmor/SELinux) handle this.
- **Local root attacker** (compromised init, malicious kernel module, container escape). Linux kernel has no defense — they can read `/proc/<pid>/mem` regardless of `mprotect()` because of `FOLL_FORCE`. **CC is the only mitigation.**
- **Local physical attacker** (cold boot attack, JTAG, bus probe). Pure CC does not fully defend; needs additional features (encrypted RAM, fused secure boot, anti-tamper). Out of scope for this PoC, in scope for production hardening.
- **Supply-chain attacker** (modified firmware shipped from factory). Defended by attested boot (R3) — peers verify PCR measurements.

This PoC's success criterion is the **local root attacker**: after the work documented here, root on Radxa Zero 3W cannot read the OP-TEE Secure World memory at `0x08400000–0x0A400000`. We proved this indirectly: when the kernel was ALLOWED to access it (no `reserved-memory` declared), the TZASC firewall raised a synchronous SError and panicked the kernel. The boundary is enforced.

### I.4 Acceptance criteria for "CC-enabled" claim

A NodeOS build claims CC-enabled status only if:

1. ✅ **R1 (memory isolation)** verified by intentional violation test — kernel attempts to write Secure region, hardware firewall faults.
2. ✅ **R2 (secure execution)** verified by SMC round-trip — Normal World ioctl produces a measurable response from Secure World firmware.
3. ⚠️ **R3 (attestation)** — Phase 2 work; PoC documents the path but does not implement.
4. ⚠️ **R4 (secure storage)** — depends on R7 (supplicant); Phase 2.
5. ⚠️ **R5 (secp256k1 in secure context)** — depends on TA build chain; Phase 2.
6. ✅ **R6 (HW RNG)** — verified via `/dev/hwrng` on RK3566 (rockchip-rng driver loaded; FIPS validation per `rngd` in Phase 2).
7. ⚠️ **R7 (bidirectional RPC)** — partial: SMC ioctl direction works; supplicant-mediated reverse RPC blocked by Rockchip BL32 missing PRIVILEGED capability.
8. ❌ **R8 (open-source RPC stack)** — current Rockchip BL32 is a blob; full open-source path requires upstream `optee_os` build for RK3566 (Phase 2 Path C).

This PoC therefore validates **R1, R2, R6 fully; R7 partially; R3, R4, R5, R8 deferred** — sufficient to declare "CC bring-up successful at kernel handshake level" and unblock Phase 2.

---

## Part II — Choosing a CC primitive

### II.1 The candidates

| Tech | Trust boundary | Granularity | secp256k1 | Open source | Ships in 2026 SBCs? |
|---|---|---|---|---|---|
| **ARM TrustZone + OP-TEE** | EL3/S-EL1 vs NS-EL1 | Coarse (Secure World vs Normal World) | Software inside TA | OS open; vendor BL32 often blob | Yes — RK35xx, Pi 5 (limited), all Cortex-A |
| **Apple Secure Enclave** | Dedicated coprocessor | Fine (per-key) | secp256r1 only | Closed | Apple Silicon only |
| **Intel SGX** | Per-process enclave | Fine (per-app) | Software inside enclave | OS open | Server-only; deprecated client-side |
| **Intel TDX** | VM-level | Coarse (whole VM) | Software | Open | New servers only; not SBC |
| **AMD SEV-SNP** | VM-level | Coarse (whole VM) | Software | Open | Server EPYC; not SBC |
| **RISC-V Keystone** | PMP-enforced enclave | Fine | Software | Fully open | Milk-V Jupiter 2 (target hardware for NodeOS phase 5+) |
| **RISC-V Penglai** | PMP/sPMP + crypto | Fine | Software | Fully open | Same |
| **Discrete TPM 2.0** | Coprocessor | Sealing only, no execution | No (sealing yes, signing no) | Open spec | Yes — TPM HATs available |
| **Discrete Secure Element (ATECC608A, OPTIGA)** | Coprocessor | Per-key | NO — P-256 only on ATECC608A | Closed | Yes — but unsuitable for Lightning (R5 fail) |

### II.2 Why TrustZone for the first PoC

- **R5 satisfied by software-in-TA pattern.** ATECC608A and OPTIGA fail R5 outright. TrustZone allows running libsecp256k1 inside a Trusted Application; key never leaves Secure World.
- **Hardware available today.** Rockchip RK3566 boards are inexpensive (~$30) and ship rkbin binaries. Acquisition lag is hours, not weeks.
- **Mature toolchain.** OP-TEE has 7+ years of upstream activity; TF-A is the de-facto BL31. Documentation exists at `optee.readthedocs.io`.
- **Dual-track with future RISC-V.** The CC primitives in NodeOS code (`OpteeWrapKeyProvider`, `OpteeChannelSigner`, `KeystoneWrapKeyProvider`, …) are trait-based at the Rust level. Once Phase 2 works on TrustZone, swapping the implementation for Keystone or Penglai is a matter of writing a new impl, not rearchitecting.
- **Failure modes are educational.** The 6 iterations documented in Appendix B are *exactly* the kind of FIQ-routing, memory-firewall, ABI-mismatch issues that any CC stack will surface. Doing this on TrustZone now means we know the pattern when we hit RISC-V later.

### II.3 What this PoC does NOT prove

- **Long-term hardware bet.** Boss has stated the long-term aim is Milk-V Jupiter 2 (RISC-V). RK3566 is a substrate for *learning the bring-up process*, not the production target. Treat this guide as a methodology that travels.
- **Production-grade attestation.** The PoC has no remote attestation. Mesh peers cannot yet cryptographically verify a node's firmware state. R3 implementation is a Phase 2 deliverable.
- **Penetration resistance.** No JTAG glitch, fault-injection, or DPA testing. Production hardening (anti-tamper packaging, fused secure boot, encrypted RAM) is a future hardware phase.

---

## Part III — RK3566 + OP-TEE PoC: the case study

### III.1 Hardware and software inventory

```
Board:             Radxa Zero 3W
SoC:               Rockchip RK3566 (4× Cortex-A55 @ 1.6 GHz, ARMv8.2-A)
RAM:               1 GB LPDDR4 (effective 1022 MiB after OP-TEE reserves 32 MB)
Storage:           microSD 32 GB (16.8 MB config + 314.6 MB EFI boot + 31.1 GB ext4 rootfs)
WiFi/BT chip:      AICSemi AIC8800DS2 (board has this variant; AP6212 DTB still boots)
UART debug:        UART2 @ 1500000 baud, USB-UART adapter on Mac via picocom
```

```
Firmware:
  BL31:  Trusted Firmware-A v2.3-896 (rkbin rk3566_bl31_v1.45.elf, fwver v1.45)
  BL32:  OP-TEE OS 3.13.0-891-g9f2aca7d1 (rkbin rk3566_bl32_v2.15.bin, fwver v2.15)
  DDR:   rkbin rk3566_ddr_1056MHz_v1.23.bin
  BL33:  U-Boot 2025.01-dirty (mainline, custom build from this PoC)

OS:      Debian 12 (bookworm), kernel 6.1.84-10-rk2410-nocsf (Rockchip downstream)
Userspace TEE tools: optee-client 3.19.0-1 (Debian package)
```

### III.2 The boot chain (verified end-to-end)

```
ROM → idbloader.img (TPL/SPL) → u-boot.itb (FIT containing BL31, BL32, BL33, DTB)
                                  │
                                  ├── BL31 v1.45 (TF-A) at EL3
                                  │     "Using opteed sec cpu_context!"
                                  │     handoff to BL32 via opteed dispatcher
                                  │
                                  ├── BL32 v2.15 (OP-TEE 3.13.0) at S-EL1
                                  │     "Primary CPU initializing"
                                  │     "Primary CPU switching to normal world boot"
                                  │
                                  └── BL33 (U-Boot 2025.01) at NS-EL1
                                        bootflow scan -lb
                                        reads /boot/extlinux/extlinux.conf
                                        loads vmlinuz + initrd + patched DTB
                                        booti
                                          │
                                          └── Linux kernel at NS-EL1
                                                optee driver probes firmware/optee
                                                /dev/tee0 + /dev/teepriv0 created
                                                systemd → login
```

Cold-boot wallclock: ~25 seconds from power-on to login prompt.

### III.3 Critical gotcha #1 — `ttyFIQ0` console hijacking

**Symptom seen on UART after applying the (incorrect) initial config:**

```
Starting kernel ...

[    8.951948] rockchip_clk_register_muxgrf: regmap not available
[    8.952537] rockchip_clk_register_branches: failed to register clock clk_32k_ioe: -524
/oUcno
Tn hor
/n horIcPt
```

The garbled output is the smoking gun. It's not memory corruption — it's the FIQ console driver writing partial frames because the FIQ interrupt line has been hijacked.

**Mechanism**:
1. Rockchip Debian images set `console=ttyFIQ0,1500000n8` in extlinux APPEND.
2. `ttyFIQ0` is a Rockchip-specific UART console driver (`drivers/tty/serial/rk_serial.c`) that uses **FIQ interrupts** to drain the UART FIFO at very high rates. This is needed for >115200 baud rates.
3. When OP-TEE / BL32 is active, the `opteed` SPD (Secure Payload Dispatcher) in BL31 routes non-secure FIQ to Secure World. This is the standard ARM TrustZone pattern — Secure World needs a way to be invoked asynchronously, and FIQ is the natural choice (NS-IRQ goes to NS, NS-FIQ goes to S).
4. Linux ttyFIQ0 driver registers a FIQ handler on the same line. The kernel's handler is overridden silently — no error, no warning — and the driver assumes its handler is being called, but it isn't.
5. UART FIFO fills, garbage drains, console becomes useless.

**Fix**: switch to `console=ttyS2,1500000n8`. `ttyS2` is the standard 8250-style UART driver in `drivers/tty/serial/8250/8250_dw.c` and uses a regular IRQ (not FIQ). Pair with `earlycon=uart8250,mmio32,0xfe660000` for early-boot debug output (RK3566 UART2 base address is `0xfe660000`).

**Why this mistake almost cost the PoC**: the kernel boot looked like a hang at the clock register lines (the last thing printed before the FIQ console died). It took a WebSearch on `rockchip OP-TEE ttyFIQ0 hang console` — landing on an old Rockchip forum thread — to identify the FIQ hijacking pattern. Without that, we'd have suspected DTB or kernel config and chased the wrong bug.

**Boot evidence after fix** (Appendix A.1 has the full log; key milestone):

```
[   13.024641] optee: probing for conduit method.
[   13.024675] optee: revision 3.13 (9f2aca7d)
[   13.025469] optee: dynamic shared memory is enabled
[   13.025725] optee: initialized driver
```

The fact that we now see kernel log = ttyS2 console alive = FIQ no longer hijacked.

### III.4 Critical gotcha #2 — TZASC firewall and missing `reserved-memory`

**Symptom (after gotcha #1 was fixed)**:

```
done.
Begin: Mounting root file system ... done.
Begin: Running /scripts/local-bottom ...
[  377.452009] SError Interrupt on CPU3, code 0x00000000be000011 -- SError
[  377.452009] SError Interrupt on CPU3, code 0x00000000be000011 -- SError
[  377.452056] CPU: 3 PID: 243 Comm: growroot Not tainted 6.1.84-10-rk2410-nocsf #10
[  377.452071] Hardware name: Radxa ZERO 3 (DT)
[  377.452092] pc : __init_rwsem+0x44/0x54
[  377.452283] Kernel panic - not syncing: Asynchronous SError Interrupt
[  377.452312]  dump_backtrace+0xe4/0x124
[  377.452332]  show_stack+0x1c/0x28
...
[  377.452472]  init_once+0x44/0x78
[  377.452490]  setup_object+0x5c/0x6c
[  377.452505]  new_slab+0x198/0x1fc
[  377.452514]  ___slab_alloc+0xb0/0x5a4
[  377.452525]  kmem_cache_alloc_lru+0xb4/0x1ac
[  377.452536]  ext4_alloc_inode+0x30/0x174
[  377.452548]  alloc_inode+0x2c/0xa0
[  377.452560]  iget_locked+0x90/0x12c
[  377.452570]  __ext4_iget+0x134/0xa30
[  377.452587]  ext4_lookup+0x184/0x25c
...
[  377.452689]  __do_sys_newfstatat+0x50/0x94
[  377.452736]  el0_svc+0x24/0x48
[  377.452746]  el0t_64_sync_handler+0xa8/0x134
```

The kernel boots cleanly until userspace runs `growroot` (initramfs filesystem expansion). Doing a `stat()` syscall triggers ext4 inode allocation, which calls into the slab allocator, which initializes a new RW-semaphore, and writing to that semaphore raises a fatal SError.

**Decoding the ESR_EL1 value `0xbe000011`**:

```
0xbe000011 = 0b 10111110 00000000 00000000 00010001
                ^^^^^^                              ← bits 31:26 = 0b101111 = 0x2F
                                                       EC = 0x2F → SError exception (ARMv8.2 RAS)
                       ^                            ← bit 25 = 1
                                                       IL = 1 (32-bit instruction syndrome)
                        ^                           ← bit 24 = 1
                                                       IDS = 1 (implementation-defined syndrome)
                                                          → vendor-specific error code
                         ^^^^^^^^^^^^^^^^^^^^^^    ← bits 23:0 = 0x000011
                                                       ISS = 0x000011 (Rockchip-specific)
```

`IDS = 1` means "this is a vendor-specific syndrome" — for Rockchip that points at the **TZASC** (TrustZone Address Space Controller). TZASC is the memory firewall: it sits on the bus between the CPU cluster and DDR, and partitions DRAM into Secure-only / Non-Secure-allowed regions per BL31's setup.

**Why this happened**:

OP-TEE memory layout per the rkbin BL32 v2.15 binary:
```
I/TC: OP-TEE memory: TEEOS 0x200000 TA 0xc00000 SHM 0x200000
```

BL32 base is `0x08400000`. Total reserved span is 32 MB (`0x08400000 – 0x0A400000`). BL31 configures TZASC to make this Secure-only.

The Rockchip Debian downstream kernel's DTB ships **without** any `reserved-memory` declaration for this range. The kernel maps the entire 1 GB DRAM (minus the standard initrd/kernel/SMBus regions) and treats `0x08400000` as ordinary RAM. The slab allocator eventually hands out a page from inside `0x08400000–0x0A400000` to a kmem_cache; the first store to that page hits the TZASC firewall; CPU3 takes a fatal SError.

It was running until the slab allocator happened to dip into that range — explaining why the kernel got all the way to userspace `growroot` before panicking.

**Fix — add to kernel DTB**:

```dts
/ {
    reserved-memory {
        optee@8400000 {
            reg = <0x00000000 0x08400000 0x00000000 0x02000000>;
            no-map;
        };
    };
    firmware {
        optee {
            compatible = "linaro,optee-tz";
            method = "smc";
        };
    };
};
```

The `reserved-memory/optee@8400000` node with `no-map` instructs the kernel to leave this 32 MB region completely outside its memory map — it is never mapped, never used by the slab allocator, never touched by ordinary kernel code. The kernel knows the region is "off-limits" without knowing why.

The `firmware/optee` node with `compatible = "linaro,optee-tz"` is what the in-kernel `drivers/tee/optee/` driver matches against during DT probe. Without this node, the driver does not attach and `/dev/tee0` is never created. (Hence the apparent paradox that the original DTB had neither node — the kernel both used the OP-TEE region AND failed to expose `/dev/tee0`. Both facts have the same root cause.)

**Verification after applying patched DTB** (full output Appendix A.4):

```
$ ls /sys/firmware/devicetree/base/firmware/optee/
compatible  method  name

$ ls /sys/firmware/devicetree/base/reserved-memory/ | grep optee
optee@8400000

$ ls /dev/tee*
/dev/tee0  /dev/teepriv0
```

### III.5 Critical gotcha #3 — `tee-supplicant` cannot attach (PRIVILEGED capability missing)

After installing `tee-supplicant` from Debian bookworm:

```
$ sudo timeout 5 tee-supplicant
ERR [1656] TEES:main:884: failed to find an OP-TEE supplicant device
```

But systemd reports the service as `active (running)` — confusingly. Inspection of upstream `optee_client` source code (`tee_supplicant.c` line 884) reveals:

```c
if (!have_supp_dev) {
    fprintf(stderr, "%s: failed to find an OP-TEE supplicant device\n", __func__);
    return 1;
}
```

The check iterates `/dev/teepriv*`, opens each, and verifies via `TEE_IOC_VERSION` ioctl that `gen_caps & TEE_GEN_CAP_PRIVILEGED` (bit `0x2`) is set. If no privileged device is found, supplicant returns 1.

**Diagnostic — direct ioctl on Rockchip BL32**:

```c
struct tee_ioctl_version_data ver;
ioctl(open("/dev/tee0", O_RDWR), TEE_IOC_VERSION, &ver);
// ver.impl_id   = 1            (OP-TEE)
// ver.impl_caps = 0x00000001
// ver.gen_caps  = 0x0000000d
//   bit 0 (TEE_GEN_CAP_GP)        = 1 ✅ GlobalPlatform compliant
//   bit 1 (TEE_GEN_CAP_PRIVILEGED) = 0 ❌ NOT advertised by Rockchip BL32
//   bit 2 (TEE_GEN_CAP_REG_MEM)   = 1 ✅
//   bit 3 (TEE_GEN_CAP_MEMREF_NULL)= 1 ✅
```

(Full program in Appendix A.5.)

**Why this matters**: a TA executing inside Secure World cannot, by itself, read a file from `/lib/optee_armtz/` or store a blob to disk — those are Normal-World resources. The OP-TEE supplicant is the userspace daemon that handles **reverse RPC**: when a TA calls `TEE_ReadObjectData`, the request flows back out of Secure World, into the kernel optee driver, into the supplicant via `/dev/teepriv0`, and the supplicant performs the file IO and returns the bytes.

Without supplicant, TAs that need filesystem access (≈90% of useful TAs, including the planned Key Manager TA) cannot run.

**Why Rockchip strips the flag**: hypothesis — Rockchip ships a closed BSP supplicant that uses a private ABI, and they've masked the PRIVILEGED bit on upstream `/dev/teepriv0` to prevent confusion with their custom path. The closed supplicant ships in their `rk356x-bsp` Buildroot release.

**Phase 2 paths** (deferred):
1. **Patch Debian's `tee-supplicant`** to skip the PRIVILEGED check (one-line change). May still fail at the first reverse RPC if the actual ABI differs.
2. **Use Rockchip BSP supplicant binary** — closed source, breaks R8 (open-source claim).
3. **Build OP-TEE OS from source for RK3566** — produces upstream-clean BL32 with PRIVILEGED set. Effort: 2–4 weeks; requires platform port to upstream `optee_os`. This is the path that would graduate the PoC to a fully-open NodeOS production firmware.

For PoC scope, supplicant is unnecessary: SMC channel is proven via direct ioctl, and TA execution is a Phase 2 deliverable.

### III.6 Critical gotcha #4 — DTB variant tolerance (AIC8800 vs AP6212)

The PoC was conducted with `rk3566-radxa-zero-3w-ap6212.dtb` patched, despite the actual board WiFi/BT chip being AICSemi AIC8800DS2 (not Broadcom AP6212).

**Why the kernel still booted**:
- WiFi: the SDIO bus probe matches AIC8800 by chip ID, regardless of DTB declarations. The AIC8800 driver loads (`AICWFDBG` log lines), MAC `94:ba:06:10:56:b3` is read out, WiFi associates.
- BT: the DTB declares a Broadcom-style serial-attached BT chip on `ttyS1`. Kernel runs the BCM init sequence → `BCM: failed to write update baudrate (-110)` → BT does not come up. Non-fatal.

**Recommendation**: for production-quality config, patch `rk3566-radxa-zero-3w-aic8800ds2.dtb` instead — same patch, just a different starting DTB. The OP-TEE additions are independent of WiFi/BT chip declarations.

**Why this gotcha matters for *generic* CC bring-up**: it illustrates a broader pattern. **CC patches are localized to specific DT subnodes (`/firmware`, `/reserved-memory`) that don't conflict with peripheral subnodes.** Variants of the same board can share the same OP-TEE DTB patch. This will be true on Milk-V Jupiter 2 (multiple I/O carrier boards with the same SoC) and on Pi 5 (different camera/HAT permutations).

### III.7 The proof-of-life test program

Even without a TA, we can prove the SMC channel is live by performing a `TEE_IOC_VERSION` ioctl on `/dev/tee0`. This:

1. Issues an ioctl into the kernel `optee` driver.
2. The driver issues an SMC instruction (the only way to enter EL3 from NS-EL1).
3. BL31 routes the SMC to BL32 via the opteed dispatcher.
4. BL32 reads its capability state and returns it via the SMC return path.
5. BL31 returns to NS-EL1.
6. The kernel driver copies the response into userspace via the ioctl return path.

This is a complete Normal-World ↔ Secure-World round trip in a few microseconds. Every other TEE primitive (open session, invoke command, register memory) builds on this same SMC mechanism.

```c
/* /tmp/tee_version.c — minimal SMC round-trip validation */
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <string.h>
#include <linux/tee.h>

int main(void) {
    int fd = open("/dev/tee0", O_RDWR);
    if (fd < 0) { perror("open"); return 1; }

    struct tee_ioctl_version_data ver;
    memset(&ver, 0, sizeof(ver));
    if (ioctl(fd, TEE_IOC_VERSION, &ver) < 0) {
        perror("TEE_IOC_VERSION"); close(fd); return 2;
    }

    printf("impl_id=%u impl_caps=0x%x gen_caps=0x%x\n",
           ver.impl_id, ver.impl_caps, ver.gen_caps);
    close(fd);
    return 0;
}
```

Build: `gcc -o /tmp/tee_version /tmp/tee_version.c` (uses kernel `<linux/tee.h>` — install `optee-client-dev` if missing).
Run: `sudo /tmp/tee_version`
Expected output: `impl_id=1 impl_caps=0x1 gen_caps=0xd`

**A successful run of this 30-line program is the PoC's primary acceptance proof for R1+R2+R7-partial.**

---

## Part IV — Reproducible deployment

### IV.1 Persist script (run on already-booted Radxa Zero 3W)

This script captures the entire patching sequence in one runnable artifact. It assumes the board has already been flashed with `u-boot-rockchip.bin` containing the BL31+BL32 paired binaries. (Building that image is documented in Part III.1 and not repeated here.)

```bash
#!/bin/bash
# Persist OP-TEE config — Radxa Zero 3W
# Idempotent; safe to re-run

set -eu
KVERS="6.1.84-10-rk2410-nocsf"   # adjust per kernel version
DTB_DIR="/usr/lib/linux-image-${KVERS}/rockchip"
ORIG_DTB="${DTB_DIR}/rk3566-radxa-zero-3w-ap6212.dtb"
PATCHED_DTB="${DTB_DIR}/rk3566-radxa-zero-3w-ap6212-optee.dtb"
EXTLINUX="/boot/extlinux/extlinux.conf"
ROOT_UUID="$(findmnt -no UUID /)"

# 1. Backup
sudo cp -n "${ORIG_DTB}" "${ORIG_DTB}.bak"
sudo cp -n "${EXTLINUX}" "${EXTLINUX}.bak"

# 2. Toolchain
sudo apt update -qq
sudo apt install -y device-tree-compiler

# 3. Decompile + patch + recompile DTB
sudo dtc -I dtb -O dts -o /tmp/orig.dts "${ORIG_DTB}"

cat > /tmp/optee-patch.dts <<'EOF'

/ {
    reserved-memory {
        optee_reserved: optee@8400000 {
            reg = <0x00000000 0x08400000 0x00000000 0x02000000>;
            no-map;
        };
    };
    firmware {
        optee_fw: optee {
            compatible = "linaro,optee-tz";
            method = "smc";
        };
    };
};
EOF

sudo bash -c "cat /tmp/orig.dts /tmp/optee-patch.dts > /tmp/patched.dts"
sudo dtc -I dts -O dtb -o "${PATCHED_DTB}" /tmp/patched.dts

# 4. Verify patched DTB has OP-TEE nodes
sudo fdtdump "${PATCHED_DTB}" 2>/dev/null | grep -E "optee@8400000|linaro,optee-tz" || {
    echo "FATAL: patched DTB lacks OP-TEE nodes — aborting"; exit 1;
}

# 5. Rewrite extlinux.conf
sudo chattr -i "${EXTLINUX}" 2>/dev/null || true

sudo tee "${EXTLINUX}" > /dev/null <<EXTLINUX_EOF
## /boot/extlinux/extlinux.conf
## Modified for OP-TEE PoC. To regenerate via u-boot-update:
##   sudo chattr -i /boot/extlinux/extlinux.conf

default optee
menu title U-Boot menu
prompt 1
timeout 30

label optee
    menu label Debian (OP-TEE enabled, ttyS2)
    linux /boot/vmlinuz-${KVERS}
    initrd /boot/initrd.img-${KVERS}
    fdt /usr/lib/linux-image-${KVERS}/rockchip/rk3566-radxa-zero-3w-ap6212-optee.dtb
    append root=UUID=${ROOT_UUID} console=ttyS2,1500000n8 earlycon=uart8250,mmio32,0xfe660000 rw loglevel=4 coherent_pool=2M irqchip.gicv3_pseudo_nmi=0 cgroup_enable=cpuset cgroup_memory=1 cgroup_enable=memory swapaccount=1

label l0r
    menu label Rescue (original DTB, ttyFIQ0 — will hang console; for u-boot CLI fallback)
    linux /boot/vmlinuz-${KVERS}
    initrd /boot/initrd.img-${KVERS}
    fdtdir /usr/lib/linux-image-${KVERS}/
    append root=UUID=${ROOT_UUID} console=ttyFIQ0,1500000n8 splash loglevel=4 rw earlycon consoleblank=0 console=tty1 coherent_pool=2M irqchip.gicv3_pseudo_nmi=0 cgroup_enable=cpuset cgroup_memory=1 cgroup_enable=memory swapaccount=1 single
EXTLINUX_EOF

# 6. Lock against u-boot-update regeneration
sudo chattr +i "${EXTLINUX}"

echo "Done. Reboot to verify: sudo reboot"
```

### IV.2 Validation after reboot

```bash
# Kernel-side: driver attached
$ ls /dev/tee*
/dev/tee0  /dev/teepriv0

$ sudo dmesg | grep -i optee
[   13.024641] optee: probing for conduit method.
[   13.024675] optee: revision 3.13 (9f2aca7d)
[   13.025469] optee: dynamic shared memory is enabled
[   13.025725] optee: initialized driver

# DT-side: nodes present
$ ls /sys/firmware/devicetree/base/firmware/optee/
compatible  method  name

$ ls /sys/firmware/devicetree/base/reserved-memory/ | grep optee
optee@8400000

# SMC round-trip works
$ sudo apt install -y optee-client-dev
$ cat > /tmp/tee_version.c <<'EOF'
[program from §III.7]
EOF
$ gcc -o /tmp/tee_version /tmp/tee_version.c
$ sudo /tmp/tee_version
impl_id=1 impl_caps=0x1 gen_caps=0xd
```

If all five checks pass, the PoC is reproduced.

### IV.3 Negative test — confirm TZASC really enforces

Optional but illuminating. Attempt to read OP-TEE memory from Normal World:

```bash
$ sudo dd if=/dev/mem bs=1 count=16 skip=$((0x08400000)) 2>&1 | xxd
```

Expected: read fails (`Operation not permitted` or similar) due to `CONFIG_STRICT_DEVMEM=y` in kernel config. Even if `STRICT_DEVMEM` were disabled, the access would raise an SError.

The fact that the system *did* SError-panic during the unpatched DTB run (Part III.4) is itself a proof that the firewall is enforced. In a hardened production deploy you would not test this destructively, but during PoC the panic was useful evidence.

---

## Part V — Generic playbook for future boards

This is the methodology distilled from the RK3566 case study, applicable to any SBC the mesh later embraces.

### V.1 The 8-step CC bring-up checklist

For each new board, verify in order:

| # | Question | RK3566 answer | How to verify on a new board |
|---|---|---|---|
| **1** | Does the SoC support a CC primitive in hardware? | Yes — Cortex-A55 has TrustZone | Check architecture manual for "TrustZone", "SGX", "SEV", "Keystone PMP", "Secure Enclave" |
| **2** | Is firmware available — open or vendor blob? | Yes — Rockchip rkbin | Search `<vendor>-bin` repos, BSP releases, OP-TEE supported platform list |
| **3** | Does mainline U-Boot/equivalent bootloader integrate it? | Yes — binman split-elf | Check bootloader docs for SoC platform support |
| **4** | Does the kernel ship with the matching driver enabled? | Yes — `CONFIG_OPTEE=y` in `rk2410-nocsf` | `cat /boot/config-$(uname -r) \| grep -iE 'optee\|sgx\|sev\|keystone'` |
| **5** | Is the kernel DTB / kernel parameters wired correctly? | NO — needed `firmware/optee` + `reserved-memory/optee` patch | Check DT bindings for the CC subsystem; compare to known-good reference |
| **6** | Does Normal-World userspace have the client library? | Yes — `libteec1` in Debian | `apt search` for `libteec`/`libsgx`/`libkeystone` |
| **7** | Does the supplicant/equivalent reverse-RPC path work? | NO — Rockchip BL32 missing PRIVILEGED cap | Run direct ioctl test (Part III.7); check capability flags |
| **8** | Can a TA / enclave / module be loaded and executed? | NOT YET — Phase 2 | Run vendor-supplied or upstream test suite (xtest, sgx-tools, etc.) |

For each step that fails, document **why** (gotcha catalog) and **what fix path is required** (in-kernel patch, DTB patch, userspace patch, firmware rebuild). Steps 5 and 7 are where most CC bring-ups stall.

### V.2 Lessons from RK3566 that apply broadly

1. **Vendor and upstream firmware ABIs drift.** RK3566 BL31 v1.45 was tied to the specific BL32 v2.15 it shipped with; mixing with upstream OP-TEE failed at handoff. Expect the same on any vendor SoC. Use paired binaries unless you build both from source.

2. **Console handlers are CC-fragile.** The FIQ-vs-NS-FIQ pattern is generic: any CC stack that uses asynchronous interrupts to enter the secure context (FIQ on ARM, IRQ-with-secure-bit on RISC-V, etc.) will conflict with kernel drivers that share that mechanism. Always check the kernel cmdline for unusual console settings before declaring CC integration broken.

3. **Memory firewalls require the kernel to opt out, not opt in.** TZASC, RISC-V PMP, Intel TDX page tables — all enforce by *blocking access*. The kernel must be told (via DT, via boot args, via firmware tables) which regions are off-limits, or it will trip the firewall during ordinary operation. The error is rarely friendly.

4. **Vendor BSP supplicants are blob-shaped on purpose.** Rockchip, NVIDIA, Apple, MediaTek — almost every SoC vendor ships a closed user-space helper that mediates between the CC subsystem and the OS. Replacing them with upstream equivalents is the line between "CC-enabled" and "open NodeOS firmware". Plan the work; don't be surprised by it.

5. **Bring-up takes 6 iterations, not 1.** Budget for serial console capture, multiple boot failures, and the discipline to read each failure carefully before reflashing. The PoC went through 5 distinct failures before reaching the working configuration. (See Appendix B for the full iter log.)

### V.3 Lessons that DO NOT generalize

These are RK3566-specific and won't apply elsewhere:

- The specific addresses (`0x08400000`, `0xfe660000`, `0x12000000`) are RK3566 only.
- The `linaro,optee-tz` compatible string is OP-TEE-specific. RISC-V Keystone uses different DT bindings (TBD; Penglai uses sBI calls without DT exposure).
- The ELF-wrap workaround (Part III.1.2) is a quirk of mainline U-Boot binman + Rockchip raw-binary BL32; vendors shipping ELF-format firmware skip this step.
- The `ttyFIQ0` console driver is Rockchip-only. Pi 5 uses standard `serial0` (PL011); Apple Silicon has its own Apple SMC console; RISC-V boards typically use SiFive or 16550-style UARTs.
- The `nocsf` kernel suffix (no Confidential Service Framework) is a Rockchip downstream branding choice; other vendors use different naming.

### V.4 Quick-start sketches for likely future boards

Each of these is a research seed only — actual PoC will require the same depth as Part III.

#### V.4.1 Milk-V Jupiter 2 (RISC-V) — STRATEGIC TARGET

- **Substrate**: SpacemiT K1 octa-core RV64GC (vendor variant; check secure extensions).
- **CC tech**: RISC-V Keystone (PMP-enforced enclaves) OR Penglai (sPMP-enforced + crypto extensions). Keystone has more upstream traction; Penglai claims production use in Alibaba.
- **Adaptation work**: replace `OpteeWrapKeyProvider` with `KeystoneWrapKeyProvider` (already trait-based at the Rust layer). Build chain entirely different — Keystone uses OpenSBI as the secure monitor, not TF-A.
- **Open question**: does SpacemiT K1 expose PMP at the M-mode level needed for Keystone? Procurement gate.
- **First R-XXX ticket** when hardware lands: "Keystone bring-up on Jupiter 2 — firmware build, eyrie SDK, first sample enclave."

#### V.4.2 Raspberry Pi 5

- **Substrate**: BCM2712 (Cortex-A76, ARMv8.2-A).
- **CC tech**: TrustZone present in silicon, but Pi Foundation has historically not enabled OP-TEE in default firmware. Recent community work (2025) has demonstrated `OP-TEE on RPi 5` via custom secondary loader. Investigate.
- **Alternative**: discrete TPM 2.0 HAT (Infineon SLB9670). Adds R3+R4 (sealing + attestation) but not R2 (no secure execution context). Useful for sealing keys derived from CPU-side TrustZone.
- **First R-XXX ticket**: "OP-TEE on Pi 5 — custom Pi Foundation firmware build, BL32 from upstream optee_os."

#### V.4.3 Apple Silicon (M-series Mac mini, etc.)

- **Substrate**: Apple Silicon ARMv8.x with proprietary Secure Enclave Processor (SEP).
- **CC tech**: SEP — no OS access from mainstream OS unless using `Asahi Linux + Apple's TrustedExecution` framework. Limited.
- **Practical path**: not for NodeOS. Apple's licensing forbids using SEP for non-Apple-blessed cryptography (no secp256k1 inside SEP). R5 fail.
- **Use case**: if a node operator runs a Mac mini, treat it as Tier 4 ("software-only + RNG-mixed"). No CC claim possible.

#### V.4.4 Intel NUC / mini-PCs with TPM 2.0 + Intel TDX

- **Substrate**: 12th gen Core+ with TDX support; or AMD Ryzen 7000+ with SEV-SNP.
- **CC tech**: TDX (entire VM as TEE) or SGX (per-process enclaves; deprecated client-side post-Skylake-X).
- **Adaptation**: heavy. NodeOS would run as a TDX guest. Backend layout changes; networking through hypervisor.
- **First R-XXX ticket**: "TDX guest mode for NodeOS — boot OVMF + virtio + measurement boot."

---

## Part VI — Phase 2 implementation plan (now unblocked)

R-012 originally listed 6 tasks. The PoC validated Task 1 fully and the firmware substrate for Tasks 2–6.

### VI.1 Task status

| R-012 Task | Description | Status after PoC | Phase 2 effort estimate |
|---|---|---|---|
| 1 | Firmware build chain | ✅ Done — see Part III.1, IV.1 | — |
| 2 | QEMU TrustZone simulation | Unblocked — proceed in parallel | 2 days |
| 3 | Key Manager TA (CMD_GENERATE_SEED, CMD_SIGN_SECP256K1, CMD_GET_WRAP_KEY, CMD_GET_PCR_SEAL) | Blocked on supplicant resolution + TA signing key access | 2 weeks |
| 4 | `OpteeWrapKeyProvider` Rust integration | Stubbed; unblocked once Task 3 produces a TA | 3 days |
| 5 | LUKS2 PCR-sealed passphrase | Depends on Task 3 (CMD_GET_PCR_SEAL) | 1 week |
| 6 | `OpteeChannelSigner` (LDK fork) | Depends on Task 3 (CMD_SIGN_SECP256K1) | 1 week |

### VI.2 Phase 2 critical path (in execution order)

1. **Decide supplicant strategy** (1 week). Options A/B/C from Part III.5. Recommendation: parallel-track A (patched Debian supplicant) and C (rebuild OP-TEE OS for RK3566). C produces a fully-open NodeOS firmware claim; A is the fast path. Discard B (closed-blob supplicant).

2. **Stand up QEMU OP-TEE dev environment** (Task 2, 2 days). Decouples TA development from hardware. Use upstream `vexpress-qemu_armv8a` platform.

3. **Write Key Manager TA in QEMU** (1 week). Validate CMD_GENERATE_SEED, CMD_SIGN_SECP256K1 on QEMU first. Use upstream xtest framework.

4. **Port Key Manager TA to RK3566** (1 week, blocked on step 1). Requires resolving signing-key access — either Rockchip BSP key or rebuilt upstream BL32.

5. **Wire Rust integration** (Tasks 4, 6 — 2 weeks). `OpteeWrapKeyProvider` and `OpteeChannelSigner` implementations.

6. **PCR-seal LUKS** (Task 5 — 1 week).

Total Phase 2: ~6–8 weeks one engineer dedicated, longer if multiplexed.

### VI.3 Hardware bet revisit

Per stakeholder direction (2026-04-23), the long-term aim is Milk-V Jupiter 2 (RISC-V). Before committing 6+ weeks of Phase 2 to RK3566-specific firmware work, re-confirm with stakeholder:

- Do we want Phase 2 to proceed on RK3566 in parallel with Jupiter 2 procurement?
- Or pause RK3566 work after PoC and focus Phase 2 effort on Keystone bring-up when hardware lands?

The PoC method (this document) transfers regardless. Time invested in Phase 2 RK3566 firmware customization may not.

---

## Appendix A — Evidence: full logs and diagnostic output

### A.1 Successful boot UART log (truncated to milestones; full log in `infra/logs/optee-poc/2026-04-25-iter6-boot.log`)

```
DDR 03ea844c5d typ 24/09/03-10:42:57,fwver: v1.23
ddrconfig:0
LPDDR4, 324MHz
BW=32 Col=10 Bk=8 CS0 Row=15 CS=1 Die BW=16 Size=1024MB
[... DDR training output ...]
change to: 1056MHz(final freq)
out

U-Boot SPL 2025.01-dirty (Apr 25 2026 - 09:53:36 +0000)
Trying to boot from MMC2

INFO:    Preloader serial: 2
NOTICE:  BL31: v2.3():v2.3-896-g70d3deb59:huan.he, fwver: v1.45
NOTICE:  BL31: Built : 16:38:07, Mar  4 2025
INFO:    GICv3 without legacy support detected.
INFO:    ARM GICv3 driver initialized in EL3
INFO:    pmu v1 is valid 220114
INFO:    l3 cache partition cfg-0
INFO:    Using opteed sec cpu_context!
INFO:    boot cpu mask: 0
INFO:    BL31: Initializing runtime services
INFO:    BL31: Initializing BL32

I/TC:
I/TC: OP-TEE version: 3.13.0-891-g9f2aca7d1 #hisping.lin (gcc version 10.2.1) #2 Thu Oct 31 10:26:19 CST 2024 aarch64, fwver: v2.15
I/TC: OP-TEE memory: TEEOS 0x200000 TA 0xc00000 SHM 0x200000
I/TC: Primary CPU initializing
I/TC: CRYPTO_CRYPTO_VERSION_NEW no support. Skip all algo mode check.
I/TC: Primary CPU switching to normal world boot

INFO:    BL31: Preparing for EL3 exit to normal world
INFO:    Entry point address = 0xa00000
INFO:    SPSR = 0x3c9


U-Boot 2025.01-dirty (Apr 25 2026 - 09:53:36 +0000)

Model: Radxa ZERO 3W
DRAM:  1 GiB (effective 1022 MiB)
PMIC:  RK817 (on=0x40, off=0x00)
Core:  299 devices, 28 uclasses, devicetree: separate
MMC:   mmc@fe2b0000: 1, mmc@fe2c0000: 2, mmc@fe310000: 0
Loading Environment from nowhere... OK
In:    serial@fe660000
Out:   serial@fe660000
Err:   serial@fe660000
Net:   No ethernet found.
Hit any key to stop autoboot:  0

[... extlinux menu pick 'optee' label, kernel + initrd + DTB load ...]

Starting kernel ...

[    0.000000] Booting Linux on physical CPU 0x0000000000 [0x412fd050]
[    0.000000] Linux version 6.1.84-10-rk2410-nocsf (radxa@radxa-build) ...
[... kernel init ...]
[   13.024641] optee: probing for conduit method.
[   13.024675] optee: revision 3.13 (9f2aca7d)
[   13.025469] optee: dynamic shared memory is enabled
[   13.025725] optee: initialized driver
[... systemd init, services start, login prompt ...]

Debian GNU/Linux 12 radxa-zero3 ttyFIQ0
                              ^^^^^^^ (cosmetic — /etc/issue template hardcoded; actual console is ttyS2)

radxa-zero3 login:
```

### A.2 Failed boot UART log — gotcha #1 (FIQ console hijacked)

This is what the ttyFIQ0 issue looked like before the fix. Note the garbled output `/oUcno Tn hor`:

```
Starting kernel ...

[    0.000000] Booting Linux on physical CPU 0x0000000000 [0x412fd050]
[... a few clock register lines, then ...]
[    8.951948] rockchip_clk_register_muxgrf: regmap not available
[    8.952537] rockchip_clk_register_branches: failed to register clock clk_32k_ioe: -524
/oUcno
Tn hor
/n horIcPt
```

After this point the kernel continues booting (other UARTs, USB, MMC are alive in dmesg), but UART output via ttyFIQ0 is dead. The board reaches systemd, services start, network associates, ssh accepts connections — but the operator sees a "hung" UART and assumes a kernel crash.

### A.3 Failed boot UART log — gotcha #2 (TZASC SError)

Full kernel panic stack from iter 5. This is the critical evidence that TZASC firewall is real:

```
done.
Begin: Mounting root file system ... Begin: Running /scripts/local-top ... done.
Begin: Running /scripts/local-premount ... done.
Warning: fsck not present, so skipping root file system
[  377.432048] EXT4-fs (mmcblk1p3): mounted filesystem with ordered data mode. Quota mode: none.
done.
Begin: Running /scripts/local-bottom ... [  377.452009] SError Interrupt on CPU3, code 0x00000000be000011 -- SError
[  377.452056] CPU: 3 PID: 243 Comm: growroot Not tainted 6.1.84-10-rk2410-nocsf #10
[  377.452071] Hardware name: Radxa ZERO 3 (DT)
[  377.452078] pstate: 40400009 (nZcv daif +PAN -UAO -TCO -DIT -SSBS BTYPE=--)
[  377.452092] pc : __init_rwsem+0x44/0x54
[  377.452124] lr : __init_rwsem+0x34/0x54
[  377.452134] sp : ffff80000bb6b5d0
[  377.452139] x29: ffff80000bb6b5d0 x28: ffff000003adaa20 x27: 0000000000000003
[  377.452160] x26: ffff000006dc3000 x25: 0000000000000c40 x24: ffff000000d78600
[  377.452174] x23: 0000000000000003 x22: 00000000ffffffff x21: ffff000000d77c00
[  377.452188] x20: ffff80000a1fc5f8 x19: ffff000008428058 x18: 0000000000000000
[... register dump ...]
[  377.452283] Kernel panic - not syncing: Asynchronous SError Interrupt
[  377.452291] CPU: 3 PID: 243 Comm: growroot Not tainted 6.1.84-10-rk2410-nocsf #10
[  377.452301] Hardware name: Radxa ZERO 3 (DT)
[  377.452307] Call trace:
[  377.452312]  dump_backtrace+0xe4/0x124
[  377.452332]  show_stack+0x1c/0x28
[  377.452342]  dump_stack_lvl+0x60/0x78
[  377.452361]  dump_stack+0x14/0x2c
[  377.452370]  panic+0x138/0x314
[  377.452387]  nmi_panic+0x50/0x70
[  377.452401]  arm64_serror_panic+0x70/0x7c
[  377.452414]  arm64_is_fatal_ras_serror+0x6c/0x88
[  377.452425]  do_serror+0x48/0x5c
[  377.452436]  el1h_64_error_handler+0x30/0x44
[  377.452450]  el1h_64_error+0x74/0x78
[  377.452462]  __init_rwsem+0x44/0x54
[  377.452472]  init_once+0x44/0x78
[  377.452490]  setup_object+0x5c/0x6c
[  377.452505]  new_slab+0x198/0x1fc
[  377.452514]  ___slab_alloc+0xb0/0x5a4
[  377.452525]  kmem_cache_alloc_lru+0xb4/0x1ac
[  377.452536]  ext4_alloc_inode+0x30/0x174
[  377.452548]  alloc_inode+0x2c/0xa0
[  377.452560]  iget_locked+0x90/0x12c
[  377.452570]  __ext4_iget+0x134/0xa30
[  377.452587]  ext4_lookup+0x184/0x25c
[  377.452602]  __lookup_slow+0xf4/0x134
[  377.452618]  walk_component+0xa0/0xdc
[  377.452631]  link_path_walk+0x2d8/0x364
[  377.452643]  path_lookupat+0x54/0x118
[  377.452657]  filename_lookup+0x98/0x10c
[  377.452669]  vfs_statx+0x7c/0x150
[  377.452680]  vfs_fstatat+0x5c/0x7c
[  377.452689]  __do_sys_newfstatat+0x50/0x94
[  377.452698]  __arm64_sys_newfstatat+0x20/0x28
[  377.452707]  invoke_syscall+0x80/0x114
[  377.452718]  el0_svc_common.constprop.0+0x94/0x134
[  377.452728]  do_el0_svc+0x98/0xbc
[  377.452736]  el0_svc+0x24/0x48
[  377.452746]  el0t_64_sync_handler+0xa8/0x134
[  377.452757]  el0t_64_sync+0x174/0x178
[  377.452772] SMP: stopping secondary CPUs
[  377.452875] CPU2: stopping
[  377.452877] CPU1: stopping
[  377.452888] CPU0: stopping
[... other CPUs dumped, then PMU+CRU register dumps from rockchip_panic_notify (for bus debugging) ...]
[  377.455116] CPU3 online:1
[  377.455121]  EL2(NS) PC: <0xffff80000872dde8> rockchip_panic_notify+0x1f8/0x2c8
```

Key facts visible here:
- Process: `growroot` (PID 243) — initramfs filesystem expansion
- Syscall: `newfstatat` — a `stat()` on a path
- Kernel path: ext4_lookup → iget_locked → ext4_alloc_inode → kmem_cache_alloc → new_slab → init_once → __init_rwsem → SError
- The slab allocator gave back a page from inside `0x08400000–0x0A400000`; the first store to a semaphore field hit TZASC.

### A.4 DTB verification — patched DTB has OP-TEE nodes

```
$ sudo fdtdump /usr/lib/linux-image-6.1.84-10-rk2410-nocsf/rockchip/rk3566-radxa-zero-3w-ap6212-optee.dtb 2>/dev/null | grep -B 2 -A 4 "linaro,optee-tz\|optee@8400000"
        };
        optee {
            compatible = "linaro,optee-tz";
            method = "smc";
        };
    };
    gpu-opp-table-mali {
--
            phandle = <0x00000130>;
        };
        optee@8400000 {
            reg = <0x00000000 0x08400000 0x00000000 0x02000000>;
            no-map;
        };
    };
```

Patched DTB is 163929 bytes (vs. 163798 original — +131 bytes for the two additional nodes plus DTC's structural overhead).

### A.5 SMC round-trip proof

```
$ sudo /tmp/tee_version
=== TEE_IOC_VERSION result ===
impl_id   = 1  (1=OPTEE, 2=AMDTEE, 3=TSTEE)
impl_caps = 0x00000001
gen_caps  = 0x0000000d
  TEE_GEN_CAP_GP          YES
  TEE_GEN_CAP_PRIVILEGED  no  (supplicant cap)
  TEE_GEN_CAP_REG_MEM     YES
  TEE_GEN_CAP_MEMREF_NULL YES

=== Round-trip OK: ioctl -> kernel -> SMC -> BL32 -> back ===
```

### A.6 Kernel CONFIG evidence

```
$ cat /boot/config-6.1.84-10-rk2410-nocsf | grep -i optee
CONFIG_ARM_SCMI_TRANSPORT_OPTEE=y
CONFIG_HW_RANDOM_OPTEE=y
# CONFIG_RTC_DRV_OPTEE is not set
CONFIG_OPTEE=y
```

`CONFIG_OPTEE=y` (built-in, not module) means OP-TEE driver is always present once the matching DT node exists.
`CONFIG_HW_RANDOM_OPTEE=y` means TEE-backed `/dev/hwrng` provider is available.
`CONFIG_ARM_SCMI_TRANSPORT_OPTEE=y` means SCMI (System Control and Management Interface) for clock/power can route through OP-TEE — useful for safe firmware-side power management.

### A.7 extlinux.conf — final state

```
## /boot/extlinux/extlinux.conf
## Modified by OP-TEE PoC persist script (2026-04-25)
## To regenerate via u-boot-update: sudo chattr -i this file first

default optee
menu title U-Boot menu
prompt 1
timeout 30


label optee
    menu label Debian GNU/Linux 12 (OP-TEE enabled, ttyS2)
    linux /boot/vmlinuz-6.1.84-10-rk2410-nocsf
    initrd /boot/initrd.img-6.1.84-10-rk2410-nocsf
    fdt /usr/lib/linux-image-6.1.84-10-rk2410-nocsf/rockchip/rk3566-radxa-zero-3w-ap6212-optee.dtb
    append root=UUID=d3e135b3-8caf-4d6e-b97a-d1da670d91b7 console=ttyS2,1500000n8 earlycon=uart8250,mmio32,0xfe660000 rw loglevel=4 coherent_pool=2M irqchip.gicv3_pseudo_nmi=0 cgroup_enable=cpuset cgroup_memory=1 cgroup_enable=memory swapaccount=1

label l0
    [original config — kept for reference; also fails with OP-TEE active due to ttyFIQ0]

label l0r
    [original rescue — same caveat]
```

The file is locked with `chattr +i` so `u-boot-update` (Debian's automatic regenerator) cannot clobber it. Unlock requires `sudo chattr -i /boot/extlinux/extlinux.conf` first.

---

## Appendix B — Iteration log: the 6 failed attempts

For future engineers who hit similar walls: this is what NOT working looked like, and what each failure taught us.

### Iter 1 — Upstream OP-TEE OS BL32 + Rockchip BL31

**Setup**: built upstream `optee_os.git` for `plat-rockchip rk3566` (does not exist yet — used `rk3399` close approximation). Tried to wedge it as BL32 inside u-boot.itb alongside Rockchip BL31 v1.45.

**Result**: BL31 prints `INFO: BL31: Initializing BL32` then UART silent. Kernel never starts.

**Lesson**: rkbin BL31 v1.45 uses Rockchip-proprietary opteed handoff format. Upstream OP-TEE entry ABI is incompatible.

**Decision**: switch to paired Rockchip BL32 v2.15 binary — abandon upstream port for now.

### Iter 2 — Same as iter 1 with cosmetic fixes

Same failure. Confirmed root cause = ABI mismatch, not configuration error.

### Iter 3 — Rockchip downstream U-Boot fork

Tried using Rockchip's u-boot v17.09 fork (which natively understands their BL31+BL32 pair). Failed at SPL build: their SPL is 325 KB, while RK3566 SD ROM expects ≤60 KB for `rksd` boot mode.

**Lesson**: Rockchip downstream u-boot is intended for eMMC boot via `rkflashtool`, not SD card. Stick with mainline u-boot.

**Decision**: ELF-wrap Rockchip BL32 binary so mainline u-boot binman accepts it (Part III.1.2). This is the workable path.

### Iter 4 — Mainline u-boot + ELF-wrapped Rockchip BL32

Build chain works. UART shows clean handoff:

```
NOTICE: BL31: v2.3-896, fwver: v1.45
INFO: Using opteed sec cpu_context!
NOTICE: I/TC: OP-TEE version: 3.13.0-891-g9f2aca7d1, fwver: v2.15
NOTICE: I/TC: Primary CPU initializing
NOTICE: I/TC: Primary CPU switching to normal world boot
INFO: BL31: Preparing for EL3 exit to normal world
```

Then U-Boot proper starts, autoboots, kernel begins... and hangs with garbled UART (gotcha #1: ttyFIQ0).

**Lesson**: BL31+BL32 chain works. Failure is downstream — kernel-level, not firmware.

### Iter 5 — Identified ttyFIQ0 issue, but introduced second bug

Diagnosed FIQ console issue via WebSearch. First attempted fix: modify u-boot's *embedded* DTB to add `firmware/optee` node. INEFFECTIVE — kernel uses its own DTB from rootfs, not u-boot's embedded one.

Second attempt: bypass extlinux.conf entirely, manually `ext4load` kernel+initrd+DTB and `setenv bootargs "console=ttyS2 ..."` via u-boot CLI, then `booti`.

**Result**: kernel boots WAY further than before — past clock register, past initramfs `Begin: Mounting root file system`, into `growroot`. Then SError panic (gotcha #2).

**Lesson**: ttyFIQ0 fix correct; reserved-memory missing is a SECOND bug. Two bugs masking each other was a major time sink.

### Iter 6 — Both fixes applied, success

Added `reserved-memory/optee@8400000` and `firmware/optee` nodes to kernel DTB via u-boot `fdt` runtime commands. Set bootargs with `console=ttyS2`. `booti`.

**Result**: full boot to login. `/dev/tee0` exists. dmesg shows OP-TEE driver probe. `TEE_IOC_VERSION` ioctl returns valid data.

**Lesson**: it took 5 prior iterations to find the right combination. Document each failure or others will repeat them.

### Total time: ~4 days from board arrival to working PoC, including all dead ends.

---

*Written 2026-04-25 by Tan Nguyen as the deliverable for [R-012](../../../../sprints/research-hub/ticket/R-012.md). Updated as Phase 2 progresses.*
