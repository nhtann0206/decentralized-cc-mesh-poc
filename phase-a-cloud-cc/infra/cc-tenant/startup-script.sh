#!/usr/bin/env bash
# Phase A tenant N2D — first-boot startup script
#
# Runs as root on first boot. Reads tenant config from GCP instance metadata,
# installs minimal desktop + attestation stack, generates SEV-SNP report, and
# exposes evidence via nginx for Bob to fetch.
#
# Metadata keys consumed:
#   - tenant-id              : opaque ID Bob tracks
#   - attestation-nonce-hex  : 64 hex chars (32 bytes), Bob-supplied, binds report_data
#   - vnc-password           : per-tenant random VNC password
#   - alice-ssh-pubkey       : Alice's SSH pubkey for tenant access
#
# Output: http://<this-tenant-ip>/cc/{report.bin, vcek.der, ask.der, ark.der, manifest.json}
#         tigervnc :1 listening on tcp:5901 with vnc-password
#         tenant user with alice-ssh-pubkey in authorized_keys

set -euo pipefail
# cloud-init runs this without HOME exported; many tools (rustup, cargo) rely on
# it. Set explicitly so `set -u` doesn't abort on $HOME expansion below.
export HOME=/root
exec > >(tee -a /var/log/cc-tenant-startup.log) 2>&1
echo "[$(date -Iseconds)] cc-tenant startup BEGIN (HOME=$HOME)"

MD="http://metadata.google.internal/computeMetadata/v1/instance/attributes"
TENANT_ID=$(curl -s -H "Metadata-Flavor: Google" "$MD/tenant-id" || echo "unknown")
NONCE_HEX=$(curl -s -H "Metadata-Flavor: Google" "$MD/attestation-nonce-hex" || true)
VNC_PASSWORD=$(curl -s -H "Metadata-Flavor: Google" "$MD/vnc-password" || echo "changeme")
ALICE_PUBKEY=$(curl -s -H "Metadata-Flavor: Google" "$MD/alice-ssh-pubkey" || echo "")

echo "[$(date -Iseconds)] tenant_id=$TENANT_ID"

# ── 1. Verify pre-baked tooling ───────────────────────────────────────────
# Phase A.5+: apt deps (xfce4 + tigervnc + nginx + libssl + xxd + jq) and the
# snpguest binary are baked into the `node-cc-tenant` Packer image, so this
# script no longer runs apt or `cargo install`. If we're booted from the
# generic Ubuntu image (e.g. the env knob is unset, fallback path), drop in
# the install steps inline so tenants still start (just slowly).
export DEBIAN_FRONTEND=noninteractive
if ! command -v xfce4-session >/dev/null || [ ! -x /usr/local/bin/snpguest ]; then
  echo "[$(date -Iseconds)] WARN: pre-baked tools missing — falling back to runtime install (~10 min)"
  apt-get update -qq
  apt-get install -y -qq \
    xfce4-session xfce4-panel xfwm4 xfce4-terminal \
    tigervnc-standalone-server tigervnc-common dbus-x11 \
    nginx \
    curl wget build-essential pkg-config libssl-dev \
    xxd jq
  if [ ! -x /usr/local/bin/snpguest ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path
    /root/.cargo/bin/cargo install --root /usr/local snpguest
  fi
fi
test -x /usr/local/bin/snpguest || { echo "FATAL: snpguest binary missing"; exit 1; }
echo "[$(date -Iseconds)] snpguest version: $(/usr/local/bin/snpguest --version)"

# ── 3. Generate attestation report bound to Bob-supplied nonce ────────────
mkdir -p /var/www/cc
cd /var/www/cc

if [ -n "$NONCE_HEX" ] && [ "${#NONCE_HEX}" -eq 128 ]; then
  # 128 hex chars = 64 bytes (SEV-SNP report_data field width). Bob's nonce.
  echo "[$(date -Iseconds)] Generating report with Bob-supplied nonce"
  echo "$NONCE_HEX" | xxd -r -p > request-data.bin
  /usr/local/bin/snpguest report report.bin request-data.bin
else
  # Fallback: random nonce (still real attestation, but Bob can't bind).
  echo "[$(date -Iseconds)] WARN: no valid nonce in metadata, using --random"
  /usr/local/bin/snpguest report report.bin request-data.bin --random
fi

# ── 4. Fetch VCEK cert chain from AMD KDS ─────────────────────────────────
echo "[$(date -Iseconds)] Fetching VCEK + ASK + ARK from AMD KDS"
mkdir -p certs
/usr/local/bin/snpguest fetch ca der ./certs milan
/usr/local/bin/snpguest fetch vcek der ./certs report.bin -p milan

# ── 5. Self-verify (sanity check before exposing) ─────────────────────────
echo "[$(date -Iseconds)] Self-verifying chain + signature"
/usr/local/bin/snpguest verify certs ./certs
/usr/local/bin/snpguest verify attestation ./certs report.bin

# ── 6. Manifest for Bob to fetch ──────────────────────────────────────────
cat > /var/www/cc/manifest.json <<EOF
{
  "tenant_id": "$TENANT_ID",
  "attestation_format": "sev-snp-amd",
  "report_path": "/cc/report.bin",
  "report_size": 1184,
  "cert_chain": {
    "vcek": "/cc/certs/vcek.der",
    "ask": "/cc/certs/ask.der",
    "ark": "/cc/certs/ark.der"
  },
  "nonce_hex": "$NONCE_HEX",
  "generated_at": "$(date -Iseconds)"
}
EOF

# ── 7. Expose via nginx (read-only static) ────────────────────────────────
cat > /etc/nginx/sites-available/cc-tenant <<'EOF'
server {
  listen 80 default_server;
  listen [::]:80 default_server;
  server_name _;
  root /var/www;
  location /cc/ {
    autoindex on;
    add_header Cache-Control "no-store";
  }
  location /healthz { return 200 "ok\n"; add_header Content-Type text/plain; }
  location / { return 404; }
}
EOF
ln -sf /etc/nginx/sites-available/cc-tenant /etc/nginx/sites-enabled/cc-tenant
rm -f /etc/nginx/sites-enabled/default
nginx -t && systemctl restart nginx

# ── 8. Tenant user + SSH key injection ────────────────────────────────────
if ! id tenant &>/dev/null; then
  useradd -m -s /bin/bash -G sudo tenant
fi
mkdir -p /home/tenant/.ssh
echo "$ALICE_PUBKEY" > /home/tenant/.ssh/authorized_keys
chmod 700 /home/tenant/.ssh
chmod 600 /home/tenant/.ssh/authorized_keys
chown -R tenant:tenant /home/tenant/.ssh

# ── 9. tigervnc per-tenant config ─────────────────────────────────────────
# Systemd units (tigervnc@.service + cc-websockify.service) are pre-baked
# into the Packer image. Here we write per-tenant bits + a systemd drop-in
# that pins the VNC config we want for THIS spawn:
#
#   - SecurityTypes None: VNC-level password is intentionally disabled.
#     Real auth gates two hops upstream — browser must hold a JWT to open a
#     WebSocket on Bob, and tenant port 6080 isn't exposed externally
#     (default-allow-internal only). Adding a VNC password adds nothing
#     because the password would have to be delivered through the same
#     JWT-gated channel anyway, and noVNC can't easily inject it pre-handshake.
#   - ConditionPathExists override (clear): the baked unit gates on
#     /home/tenant/.vnc/passwd existing, but with SecurityTypes None we don't
#     need that file at all; clear the condition so the unit always starts.
#
# Using a drop-in (instead of overwriting the baked unit file) keeps the
# Packer image generic; per-spawn config diverges only through this override.
mkdir -p /home/tenant/.vnc
cat > /home/tenant/.vnc/xstartup <<'EOF'
#!/bin/bash
unset SESSION_MANAGER DBUS_SESSION_BUS_ADDRESS
exec startxfce4
EOF
chmod +x /home/tenant/.vnc/xstartup
chown -R tenant:tenant /home/tenant/.vnc

mkdir -p /etc/systemd/system/tigervnc@tenant.service.d
cat > /etc/systemd/system/tigervnc@tenant.service.d/00-cc-override.conf <<'EOF'
[Unit]
ConditionPathExists=

[Service]
ExecStart=
# Vetted Xtigervnc flags only. `-MaxProcessorUsage` and `-CompareFB` are
# rejected by tigervnc-standalone-server on Ubuntu 24.04 → service fails to
# start. Practical bottleneck is geographic RTT, not server-side encode.
ExecStart=/usr/bin/Xtigervnc :1 -localhost no -geometry 1280x800 -depth 24 -SecurityTypes None -rfbport 5901
EOF
systemctl daemon-reload

# Kick units. tigervnc@tenant.service runs Xtigervnc + xstartup;
# cc-websockify.service bridges 6080 → localhost:5901.
systemctl restart tigervnc@tenant.service
systemctl restart cc-websockify.service

# ── 10. Done ──────────────────────────────────────────────────────────────
echo "[$(date -Iseconds)] cc-tenant startup COMPLETE"
echo "  - Attestation:    http://<external-ip>/cc/manifest.json"
echo "  - VNC websocket:  ws://<internal-ip>:6080  (Bob proxy connects here)"
echo "  - VNC raw:        :5901 (tenant user)"
echo "  - SSH:            tenant@<external-ip>"
