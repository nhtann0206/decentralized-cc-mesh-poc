#!/usr/bin/env bash
# Phase A.5 — bake CC tenant tooling into a Packer image.
#
# Runs ONCE during Packer image build (not per-tenant). Per-tenant
# initialization (read metadata, generate attestation, fetch VCEK, write
# manifest) stays in startup-script.sh because those depend on the spawn-
# time nonce + chip ID.

set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
# Force HOME=/root so rustup installs to /root/.cargo, matching the path we
# use later. Without this, sudo -E from Packer leaks HOME=/home/packer and
# rustup writes to the packer user's home — which then doesn't match the
# /root/.cargo/bin/cargo path the next step expects (script_xxx.sh:29).
export HOME=/root

echo "[$(date -Iseconds)] bake-cc-tools BEGIN (HOME=$HOME)"

# 1. Desktop env + browser + VNC + websockify + nginx + cert chain tools.
# Browser ships with the image so the rented VM is immediately useful
# ("rent → open browser") without per-spawn install. Epiphany is the GNOME
# webkit browser, available as a regular apt package — Ubuntu 24.04 routes
# `apt install chromium-browser` through snapd, which the tenant network
# can't reach (snap store blocked from this VPC), and Mozilla's official
# Firefox repo would need an extra PPA. Epiphany covers the demo "open
# browser" UX with one apt line.
apt-get update -qq
apt-get install -y -qq \
  xfce4-session xfce4-panel xfwm4 xfce4-terminal xfce4-settings xfdesktop4 \
  epiphany-browser \
  tigervnc-standalone-server tigervnc-common dbus-x11 \
  python3-websockify novnc \
  nginx \
  curl wget build-essential pkg-config libssl-dev \
  xxd jq ca-certificates

# 2. Rust toolchain (system-wide so the runtime doesn't need to repeat).
# Installs to /root/.cargo/bin; binaries we need go to /usr/local for PATH.
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path

# 3. snpguest — produces /usr/local/bin/snpguest directly.
/root/.cargo/bin/cargo install --root /usr/local snpguest

# Verify what's expected to be there at runtime.
test -x /usr/local/bin/snpguest
/usr/local/bin/snpguest --version

# 4. Bake tigervnc + websockify systemd units.
#
# tigervnc as `Type=simple` running `Xtigervnc -fg` directly avoids the
# `vncserver` Perl wrapper that exits-after-start (was causing the unit to
# time out under Type=forking). Per-tenant config (xstartup, vnc passwd,
# tenant user creation) stays in startup-script.sh — only the unit + helper
# bits are pre-baked here.

cat > /etc/systemd/system/tigervnc@.service <<'EOF'
[Unit]
Description=TigerVNC X server for %i on display :1
After=syslog.target network.target

[Service]
Type=simple
User=%i
WorkingDirectory=/home/%i
Environment=HOME=/home/%i
# Run Xtigervnc in foreground so systemd tracks lifecycle directly.
#
# -SecurityTypes None: VNC-level auth is intentionally disabled because the
# real access gate sits two hops upstream — the browser must hold a valid
# JWT to even open a WebSocket on Bob, and websockify on this VM only
# listens on the VPC-internal port 6080 (firewall blocks external 6080).
# Adding VNC password here adds nothing: the password would have to be
# delivered to the browser via the same JWT-gated channel anyway, and the
# Phase A noVNC client can't easily supply it before the handshake.
# Default Xtigervnc flags. Earlier we tried `-MaxProcessorUsage 100 -CompareFB 0`
# for perf — `-MaxProcessorUsage` is not a recognized option in tigervnc-
# standalone-server on Ubuntu 24.04 (the option list was reduced upstream),
# and Xtigervnc fast-aborts with status 1 on any unknown flag. Stick with
# vetted defaults; the practical bottleneck is geographic RTT, not server-side
# encode CPU.
ExecStart=/usr/bin/Xtigervnc :1 -localhost no -geometry 1280x800 -depth 24 \
  -SecurityTypes None -rfbport 5901
ExecStartPost=/bin/bash -c 'sleep 1 && DISPLAY=:1 sudo -u %i -E /home/%i/.vnc/xstartup &'
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/cc-websockify.service <<'EOF'
[Unit]
Description=websockify bridge — noVNC port 6080 → Xtigervnc :5901
After=network.target tigervnc@tenant.service
Wants=tigervnc@tenant.service

[Service]
Type=simple
ExecStart=/usr/bin/websockify --web=/usr/share/novnc 0.0.0.0:6080 localhost:5901
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

# Enable units so they auto-start on every spawned tenant. The
# tigervnc@tenant.service ConditionPathExists guards on
# /home/tenant/.vnc/passwd, which startup-script.sh creates per tenant.
systemctl enable tigervnc@tenant.service
systemctl enable cc-websockify.service

echo "[$(date -Iseconds)] bake-cc-tools DONE — image ready for capture"
