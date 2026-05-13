#!/usr/bin/env bash
# Phase A — populate Bob's host CC evidence directory.
#
# Run ONCE on a CC-capable host (AMD SEV-SNP guest VM, e.g. n2d-standard-2 with
# --confidential-compute-type=SEV_SNP). Output goes to $CC_HOST_DIR (default
# /var/lib/cc-host). Bob's backend reads this directory to serve provider-level
# attestation to remote renters at GET /external/workspace/cc-attestation.
#
# Pre-requisite: /usr/local/bin/snpguest installed (cc-tenant startup-script
# does this for tenants; for Bob's own host run it manually or via Packer).
#
# This script is idempotent — re-running regenerates fresh evidence with a new
# random nonce. Bob's backend re-reads files on every attestation request, so
# rotating evidence requires no backend restart.

set -euo pipefail
export HOME="${HOME:-/root}"

CC_HOST_DIR="${CC_HOST_DIR:-/var/lib/cc-host}"
SNPGUEST="${SNPGUEST:-/usr/local/bin/snpguest}"

if [ ! -x "$SNPGUEST" ]; then
  echo "FATAL: $SNPGUEST not executable. Install via:"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
  echo "  /root/.cargo/bin/cargo install --root /usr/local snpguest"
  exit 1
fi

if [ ! -e /dev/sev-guest ]; then
  echo "FATAL: /dev/sev-guest missing — host is not running as SEV-SNP guest."
  echo "Verify: dmesg | grep -i sev"
  exit 2
fi

echo "[$(date -Iseconds)] cc-host bootstrap BEGIN (dir=$CC_HOST_DIR)"

mkdir -p "$CC_HOST_DIR/certs"
cd "$CC_HOST_DIR"

# Fresh random 64-byte nonce. Phase A.0: identity nonce — no anti-replay.
# Phase A.1: bind to a per-renter challenge and rotate per request.
$SNPGUEST report report.bin request-data.bin --random
echo "[$(date -Iseconds)] report.bin = $(stat -c%s report.bin) bytes"

# Pull cert chain from AMD KDS. Cached locally; safe to re-run.
$SNPGUEST fetch ca der ./certs milan
$SNPGUEST fetch vcek der ./certs report.bin -p milan
echo "[$(date -Iseconds)] certs/ contents: $(ls certs)"

# Self-verify before publishing — failing here means the host is mis-configured
# and Bob's backend would also fail to serve verifiable evidence.
$SNPGUEST verify certs ./certs
$SNPGUEST verify attestation ./certs report.bin

# Compact manifest (mirrors the cc-tenant manifest shape). Optional — Bob's
# backend reads the binary files directly, but useful for ad-hoc curl checks.
cat > "$CC_HOST_DIR/manifest.json" <<EOF
{
  "host_attestation_format": "sev-snp-amd",
  "report_path": "report.bin",
  "report_size": $(stat -c%s report.bin),
  "cert_chain": {
    "vcek": "certs/vcek.der",
    "ask": "certs/ask.der",
    "ark": "certs/ark.der"
  },
  "generated_at": "$(date -Iseconds)"
}
EOF

# Make readable by the backend service user (which may not be root).
chmod -R a+rX "$CC_HOST_DIR"

echo "[$(date -Iseconds)] cc-host bootstrap DONE"
ls -la "$CC_HOST_DIR" "$CC_HOST_DIR/certs"
