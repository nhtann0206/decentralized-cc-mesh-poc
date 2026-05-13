# noVNC Desktop Streaming for Tenant VMs

## What this is

The piece that turned the rental flow from "your VM is up, here's an SSH key" into "click Start and your Linux desktop opens in the browser tab." Without this, the marketplace looked like an IaaS provider; with it, it looked like a consumer product.

## What it does

Three layers, each with its own quirk:

```
Renter's browser ─── noVNC client in <canvas> ───┐
                                                  │ WSS (binary RFB frames)
                                                  ▼
                                          Operator backend
                                          ┌────────────────────┐
                                          │ Axum WS handler    │
                                          │  ─ session-auth    │
                                          │    gate on upgrade │
                                          │  ─ binary RFB      │
                                          │    bidirectional   │
                                          │    proxy           │
                                          └──────────┬─────────┘
                                                     │ raw TCP (RFB)
                                                     ▼
                                          Tenant VM (libvirt domain)
                                          ┌────────────────────┐
                                          │ QEMU -vnc unix:..  │
                                          │ → websockify       │
                                          │ → Xtigervnc        │
                                          │ → xfce4-session    │
                                          └────────────────────┘
```

- **Tenant VM image** ships a desktop environment, a VNC server, and websockify baked in. The Packer image bake is what makes cold-boot fast enough for "one-click rental" to actually feel one-click (the previous runtime-install approach was 10–15 minutes; the baked image is around 60 seconds).
- **Operator backend** runs an Axum WebSocket handler that authenticates the upgrade against the session, then forwards binary RFB frames *transparently* between the renter's WebSocket and the tenant's VNC Unix socket. RFB is not a text protocol; the framing on both sides has to be wrapped/unwrapped at the TCP boundary or the renter sees corrupted pixels.
- **Renter frontend** embeds the noVNC client in the React tree and reconnects with backoff on transient disconnects.

## A note on the auth gate

The query-parameter token used in this iteration's WebSocket upgrade is a known shortcut — `?token=...` works for the first deployment but doesn't survive a serious security review (tokens leak via referrer headers, browser history, server logs). The intended next step is an httpOnly cookie flow at the WS upgrade boundary. The shortcut is documented as such, so the next person picks it up rather than discovering the issue the hard way.

## Key files

- [`infra/setup-tenant-desktop.sh`](infra/setup-tenant-desktop.sh) — the desktop bake step: XFCE4 + Chromium + websockify + tigervnc, VNC over a Unix socket (not TCP — easier auth boundary), the systemd units. Drops a previous TigerVNC-on-TCP approach in favour of QEMU's built-in VNC + websockify proxy for better isolation.

## What's not in this folder

The Rust-side WebSocket proxy logic lives in `src/workspace/service.rs` in the source tree, which I left out of `cross-node-rental/` for the same code-dump-vs-highlight reason. The architecture above plus the desktop bake script is what shows the shape; the proxy itself is a straightforward Axum `WebSocketUpgrade` plus a `tokio::io::copy_bidirectional` between the WebSocket sink and the libvirt-provided Unix socket — there is no surprise in it once the architecture is clear.
