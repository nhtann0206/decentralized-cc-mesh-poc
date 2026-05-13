#!/usr/bin/env bash
# Phase A — spawn 1 tenant N2D Confidential VM
#
# Usage:
#   ./spawn.sh <tenant-id> <alice-ssh-pubkey-file> [vnc-password]
#
# Bob's `GcpConfidentialVmProvider` (Rust, packages/backend/src/workspace/) will
# mirror this command via the GCP REST API. This shell version is for design
# verification + manual smoke test.
#
# Example:
#   ssh-keygen -t ed25519 -N '' -f /tmp/alice-test-key
#   ./spawn.sh test-001 /tmp/alice-test-key.pub demo123

set -euo pipefail

TENANT_ID="${1:?tenant-id required (e.g. test-001)}"
ALICE_PUBKEY_FILE="${2:?alice ssh pubkey file required}"
VNC_PASSWORD="${3:-$(openssl rand -hex 8)}"

PROJECT="${GCP_PROJECT:?set GCP_PROJECT to your own GCP project ID}"
ZONE="${GCP_ZONE:-us-central1-a}"
MACHINE_TYPE="${TENANT_MACHINE:-n2d-standard-2}"
IMAGE_FAMILY="${TENANT_IMAGE_FAMILY:-ubuntu-2404-lts-amd64}"
IMAGE_PROJECT="${TENANT_IMAGE_PROJECT:-ubuntu-os-cloud}"

INSTANCE_NAME="cc-tenant-${TENANT_ID}"

# Per-tenant nonce for attestation report_data binding.
# Bob backend MUST pass this so it can verify the same nonce in the returned report.
NONCE_HEX="${TENANT_NONCE:-$(openssl rand -hex 32)}"

ALICE_PUBKEY="$(cat "$ALICE_PUBKEY_FILE")"

echo "=== Phase A tenant spawn ==="
echo "  tenant_id    : $TENANT_ID"
echo "  instance     : $INSTANCE_NAME"
echo "  zone         : $ZONE"
echo "  machine_type : $MACHINE_TYPE"
echo "  nonce        : $NONCE_HEX"
echo "  vnc_password : $VNC_PASSWORD"
echo ""

gcloud compute instances create "$INSTANCE_NAME" \
  --project="$PROJECT" \
  --zone="$ZONE" \
  --machine-type="$MACHINE_TYPE" \
  --image-family="$IMAGE_FAMILY" \
  --image-project="$IMAGE_PROJECT" \
  --confidential-compute-type=SEV_SNP \
  --maintenance-policy=TERMINATE \
  --boot-disk-size=20GB \
  --boot-disk-type=pd-balanced \
  --tags=cc-tenant \
  --metadata-from-file=startup-script="$(dirname "$0")/startup-script.sh" \
  --metadata="\
tenant-id=$TENANT_ID,\
attestation-nonce-hex=$NONCE_HEX,\
vnc-password=$VNC_PASSWORD,\
alice-ssh-pubkey=$ALICE_PUBKEY,\
enable-oslogin=FALSE"

echo ""
echo "=== Waiting for instance RUNNING ==="
gcloud compute instances describe "$INSTANCE_NAME" --zone="$ZONE" \
  --format="value(status,networkInterfaces[0].accessConfigs[0].natIP)"

echo ""
echo "=== Tenant spawned. Next steps ==="
echo "  1. Wait ~5-10 min for cloud-init + snpguest install + attestation generate"
echo "  2. curl http://<EXTERNAL_IP>/cc/report.bin > tenant-report.bin"
echo "  3. snpguest verify chain + signature against fixtures"
echo "  4. Connect VNC via :5901 with password '$VNC_PASSWORD'"
echo ""
echo "  Cleanup when done:"
echo "    gcloud compute instances delete $INSTANCE_NAME --zone=$ZONE --quiet"
