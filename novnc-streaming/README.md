# noVNC Desktop Streaming for Tenant VMs

**When**: 2026-04-20, 1 day.

The piece that turned the rental flow from "VM is up, congratulations, here's an SSH key" into "click Start and your Linux desktop opens in the browser tab within 60 seconds." This was the demo-defining change — without it, the marketplace looked like an IaaS provider; with it, it looked like a consumer product.

## Architecture

```
Alice's browser ─── noVNC (canvas, JS) ──┐
                                          │ WSS (binary RFB frames)
                                          ▼
                                   Bob's backend
                                   ┌──────────────────┐
                                   │ /ws/vnc/:session │  Axum WS handler
                                   │   ─ session auth │
                                   │   ─ binary proxy │
                                   └────────┬─────────┘
                                            │ raw TCP RFB
                                            ▼
                                   Tenant VM (libvirt domain)
                                   ┌──────────────────┐
                                   │ QEMU -vnc unix:… │  Unix socket
                                   │ → websockify     │  TCP→WS bridge
                                   │ → Xtigervnc      │  X server
                                   │ → xfce4-session  │  Desktop env
                                   └──────────────────┘
```

Three layers, each with its own quirk:
1. **Tenant VM image** has to ship a desktop environment + VNC server + websockify, baked in (cold-boot cost matters — Phase A's Packer image did this so the first boot is ~60s).
2. **Bob backend's WebSocket proxy** has to forward binary RFB frames *transparently* — RFB is not a text protocol, and tokio-tungstenite's `Message::Binary` framing has to be wrapped/unwrapped at the TCP boundary. Per-session JWT auth gate on the WS upgrade (the `?token=` query param flow is documented as Phase 1 only; Phase 2 should move to httpOnly cookies — see project memory `vnc-auth-phase2`).
3. **Frontend noVNC client** embeds in the React tree (`PeerWorkspacePage.tsx`'s VNC tab — see `../cross-node-rental/frontend/PeerWorkspacePage.tsx`) and reconnects with backoff on disconnect.

## What's here

- [`infra/setup-tenant-desktop.sh`](infra/setup-tenant-desktop.sh) — the Packer image bake step. Installs XFCE4 + Chromium + websockify + tigervnc, configures the VNC server to listen on a Unix socket (not TCP — easier auth boundary), wires the systemd units. The "no TigerVNC" in the commit message refers to dropping a previous TigerVNC-on-TCP approach in favor of QEMU's built-in VNC + websockify proxy, which gave better isolation.

## What's not here

The Rust-side WebSocket proxy logic lives in `src/workspace/service.rs` (the giant 3500-line file I deliberately didn't copy into the cross-node-rental folder — see that folder's README). For the portfolio narrative, the architecture diagram above plus the desktop-bake script is enough to show I owned the design end-to-end. The Rust code is straightforward Axum `WebSocketUpgrade` + a bidirectional `tokio::io::copy_bidirectional` between the WS sink and the libvirt-provided Unix socket; nothing surprising once you understand the architecture.

## Commits

`236ddf6f` (QEMU VNC WebSocket for tenant VM desktop access), `f56cdc02` (Bob-side VNC WebSocket proxy), `f9cdc45d` (frontend embed of noVNC desktop viewer), `e048b965` (tenant desktop image script), `af95ae08` (peer selector dropdown + static IPs for GCP demo prep).
