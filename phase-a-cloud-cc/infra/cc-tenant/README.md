# GCP Confidential VM Tenant — Phase A spawn pattern

**Date**: 2026-04-28
**Phase**: A (cloud SEV-SNP marketplace, intermediate milestone)
**Architecture**: Bob (N2D Confidential VM) orchestrates per-rent tenant N2D Confidential VMs via GCP API.

## Tổng quan flow

```
Alice browser
   │ 1. POST /api/v2/workspaces { tenant_request }
   ▼
Bob (N2D, SEV-SNP)
   │ 2. gcloud compute instances create tenant-<uuid> ...
   │    --confidential-compute-type=SEV_SNP
   │    --metadata-from-file=startup-script=startup-script.sh
   │    --metadata=ssh-pubkey=<alice-pubkey>,vnc-pass=<random>
   ▼
Tenant N2D (SEV-SNP, fresh)
   │ 3. boot → cloud-init → setup-cc-tenant runs:
   │    a. apt install xfce + tigervnc + nginx + ...
   │    b. cargo install snpguest (or pre-baked Packer image)
   │    c. Generate attestation report: snpguest report report.bin --random
   │    d. Fetch VCEK + cert chain from AMD KDS
   │    e. Place at /var/www/cc/{report.bin, vcek.der, ask.der, ark.der}
   │    f. nginx serve /cc/* on port 80 (read-only)
   │    g. tigervnc :1 với password from metadata
   │    h. SSH key injected for `tenant` user
   ▼ Bob fetch attestation
   │ 4. GET http://<tenant-ip>/cc/report.bin + cert chain
   │ 5. Verify VCEK chain offline (Rust verifier in Bob backend)
   │ 6. Bind nonce to tenant_id (replay protection)
   ▼
Bob → Alice
   │ 7. Return: { tenant_url, vnc_url, vnc_password, attestation_bundle }
   ▼
Alice
   │ 8. Browser verify attestation chain client-side (defense in depth)
   │ 9. Pay LN invoice
   │ 10. Connect noVNC / SSH
```

## Files trong folder này

| File | Vai trò |
|---|---|
| `README.md` | This doc |
| `spawn.sh` | gcloud command template — Bob's `GcpConfidentialVmProvider` Rust code mirror |
| `startup-script.sh` | Inject vào tenant qua `--metadata-from-file=startup-script=...`, chạy on first boot. Cài tools + sinh attestation. |
| `cloud-init.yaml` | (alternative) cloud-init format cho startup, dùng nếu không muốn raw bash |
| `packer/node-cc-tenant.pkr.hcl` | (FUTURE) Packer config xây golden image với tools pre-installed → spawn 30-60s thay 5-10 phút. Phase A optimization, defer. |

## Boot time profile

| Strategy | First-boot time | Implementation effort |
|---|---|---|
| Stock Ubuntu 24.04 + startup-script cài tools at boot | ~5-10 phút (cargo build snpguest = 8 phút) | Low — chỉ viết bash script |
| Packer golden image với tools pre-installed | ~30-60s | +1 day Packer config + image build |

**Phase A demo recommendation**: bắt đầu strategy 1 (stock + startup-script). Nếu UX quá chậm, optimize sang Packer Phase A.5.

## Attestation freshness

Mỗi spawn = mỗi tenant có chip-unique VCEK key + nonce mới. Attacker không replay được vì:
- AMD VCEK signing key per-chip, không export được.
- Report data field bind nonce do Bob (hoặc Alice) cung cấp lúc spawn.
- TCB version trong report khớp current platform state.

Verify steps Bob phải làm sau khi spawn (Step 4 code):
1. Parse 1184-byte attestation report.
2. Fetch VCEK cert từ AMD KDS theo chip ID + TCB.
3. Verify chain VCEK ← ASK ← AMD root (ARK) — fixed hardcoded ARK pubkey.
4. Verify VCEK signed report (ECDSA P-384).
5. Check nonce trong `report_data` khớp nonce Bob đã inject.
6. Check measurement trong report khớp expected NodeOS image hash (nếu Bob ship custom image — phase A.5).

## Cost estimate per tenant rental

- Tenant N2D-standard-2: $0.0707/h × 1.4 (CC premium) ≈ **$0.10/h**
- Boot time billable từ instance start (~5-10 min stock, ~1 min Packer)
- Demo session 1-2h: **$0.10-0.20/session/tenant**

GCP credits dư thừa cho demo period ~2 tuần.

## Reference: existing `infra/gcp/packer/openclaw-desktop.pkr.hcl`

Phase 1 đã có pattern Packer build XFCE + TigerVNC image cho non-CC tenant. Phase A extend pattern này, swap base image sang Ubuntu 24.04 LTS, add snpguest + nginx-attestation-serve.

---

*Local implementation note. Commit khi spawn test pass + boss checkpoint OK.*
