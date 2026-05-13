#!/usr/bin/env bash
# Tenant VM Desktop Image — for libvirt KVM tenant VMs
#
# Installs: XFCE desktop + Chromium browser. Lightweight variant of
# setup-desktop.sh — no TigerVNC (QEMU built-in VNC exposes framebuffer
# directly), no OpenClaw, no Node.js.
#
# Base: Debian 13 trixie cloud image (generic-cloud-amd64.qcow2)
# Access: QEMU VNC → noVNC in renter's browser
# Boot: auto-login to XFCE desktop (no password prompt)
#
# Usage (build base image on GCP Bob):
#   1. Boot a temporary VM from base Debian 13 image
#   2. Run this script inside it
#   3. Shutdown + snapshot the disk as new base image
#   4. All future tenant VMs use this image → instant desktop on boot

set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

echo "=== [1/6] System update ==="
apt-get update
apt-get upgrade -y

echo "=== [2/6] Install XFCE desktop + apps ==="
apt-get install -y \
  xfce4 xfce4-terminal dbus-x11 \
  xterm htop vim curl wget \
  lightdm \
  openssh-server

# NO TigerVNC — QEMU exposes VNC from hypervisor level.
# NO Node.js, no OpenClaw — tenant decides what to install.

echo "=== [3/6] Install Chromium ==="
# Debian 13 ships chromium in main repo (not snap)
apt-get install -y chromium
echo "Chromium: $(chromium --version 2>&1 || echo 'installed')"

echo "=== [4/6] Create tenant user ==="
if id tenant &>/dev/null; then
  echo "User 'tenant' already exists"
else
  useradd -m -s /bin/bash -G sudo tenant
  # Set default password (renter can change via SSH)
  echo "tenant:tenant" | chpasswd
  echo "Created user: tenant (password: tenant)"
fi

echo "=== [5/6] Configure desktop ==="

# Auto-login: lightdm autologin for tenant user
mkdir -p /etc/lightdm/lightdm.conf.d
cat > /etc/lightdm/lightdm.conf.d/50-autologin.conf << 'EOF'
[Seat:*]
autologin-user=tenant
autologin-user-timeout=0
EOF

# Set graphical target as default
systemctl set-default graphical.target
systemctl enable lightdm

# XFCE session
echo 'xfce4-session' > /home/tenant/.xsession
chmod +x /home/tenant/.xsession
chown tenant:tenant /home/tenant/.xsession

touch /home/tenant/.Xauthority
chown tenant:tenant /home/tenant/.Xauthority

# Disable compositor (causes VNC rendering artifacts)
sudo -u tenant mkdir -p /home/tenant/.config/xfce4/xfconf/xfce-perchannel-xml
cat > /home/tenant/.config/xfce4/xfconf/xfce-perchannel-xml/xfwm4.xml << 'XFWM4'
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfwm4" version="1.0">
  <property name="general" type="empty">
    <property name="use_compositing" type="bool" value="false"/>
  </property>
</channel>
XFWM4

# Disable screensaver + lock
cat > /home/tenant/.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-screensaver.xml << 'SSCONF'
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfce4-screensaver" version="1.0">
  <property name="saver" type="empty">
    <property name="enabled" type="bool" value="false"/>
  </property>
  <property name="lock" type="empty">
    <property name="enabled" type="bool" value="false"/>
  </property>
</channel>
SSCONF

# Terminal config
sudo -u tenant mkdir -p /home/tenant/.config/xfce4/terminal
cat > /home/tenant/.config/xfce4/terminal/terminalrc << 'TERM'
[Configuration]
FontName=Monospace 12
ColorForeground=#ffffff
ColorBackground=#1a1a2e
ColorCursor=#ffffff
ColorCursorUseDefault=FALSE
TERM

chown -R tenant:tenant /home/tenant/.config

# Chromium desktop shortcut
sudo -u tenant mkdir -p /home/tenant/Desktop
cat > /home/tenant/Desktop/chromium.desktop << 'CHROME'
[Desktop Entry]
Type=Application
Name=Chromium Browser
Exec=chromium --no-sandbox %U
Icon=chromium
Terminal=false
Categories=Network;WebBrowser;
CHROME
chmod +x /home/tenant/Desktop/chromium.desktop
chown tenant:tenant /home/tenant/Desktop/chromium.desktop

echo "=== [6/6] Cleanup ==="
apt-get clean
rm -rf /var/lib/apt/lists/*

echo ""
echo "=== Tenant Desktop Image Ready ==="
echo "Desktop: XFCE 4"
echo "Browser: $(chromium --version 2>&1 || echo 'Chromium')"
echo "User: tenant (auto-login via lightdm)"
echo "SSH: enabled on :22"
echo "VNC: via QEMU built-in (no TigerVNC inside VM)"
echo ""
echo "Snapshot this disk to create the base image for tenant VMs."
