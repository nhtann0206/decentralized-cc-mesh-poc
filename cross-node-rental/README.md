# Cross-Node VM Rental — Milestones M2 → M4

**When**: 2026-04-17 → 2026-04-19, ~3 days.

The full mechanics of Alice's node renting a VM from Bob's node, billed per minute over Lightning, with the VM lifecycle persisted across pause/resume/restart. This is the "marketplace" half of the product — what Phase A then put hardware-rooted attestation on top of.

## What's here

### Backend Rust

- [`src/workspace/libvirt_provider.rs`](src/workspace/libvirt_provider.rs) (1380 lines) — **M2**. The KVM/QEMU provider. Idempotent `create` / `pause` / `resume` / `destroy` operations on libvirt domains, with stress tests verifying behavior under concurrent pause-resume cycles. The provider is the trait implementation that the rest of `workspace/` calls — swapping libvirt for GCP later (Phase A) didn't touch this layer.
- [`src/workspace/payment_timer.rs`](src/workspace/payment_timer.rs) — **M4.3b**. Alice-side BOLT12 payment timer. Watches the active session, fires the next BOLT12 offer payment ~30s before each billing tick expires, retries with backoff, hits the pause path if 3 payments fail.
- [`src/workspace/container.rs`](src/workspace/container.rs) — VM container abstraction (image, resources, network, mounts).
- [`src/endpoints/workspace_endpoint.rs`](src/endpoints/workspace_endpoint.rs) — **M4.1**. The protocol-agnostic trait. Implemented by both the server (delegates to local service) and the client (makes HTTP+L402 calls). This is what the five-layer architecture buys: a single trait that types-check across nodes.
- [`src/transport/node_client/http_workspace_client.rs`](src/transport/node_client/http_workspace_client.rs) — **M4.2**. Renter side. Implements the workspace endpoint by making HTTP requests to a peer with L402 macaroons.
- [`src/transport/node_server/http_workspace_external.rs`](src/transport/node_server/http_workspace_external.rs) — **M4.2**. Operator side. Receives the same calls and delegates to the local workspace service.
- [`src/events/handlers/workspace_payment_handler.rs`](src/events/handlers/workspace_payment_handler.rs), [`workspace_credits_handler.rs`](src/events/handlers/workspace_credits_handler.rs) — event-bus reactors. When a BOLT12 payment confirms on the LDK side, the payment handler ticks the session's paid-through timestamp; the credits handler unifies the api-store credit ledger with workspace billing.
- [`src/events/handlers/cron_command_bridge.rs`](src/events/handlers/cron_command_bridge.rs) — bridge between the cron builtin app and the workspace billing tick. Why a bridge: per project convention all recurring work goes through the cron app via `CronCommand` BusEvents — never ad-hoc `tokio::time::interval`. This handler is what made the workspace billing tick obey that rule.
- [`src/ldk_bolt12_capabilities.rs`](src/ldk_bolt12_capabilities.rs) — BOLT12 offer creation/lookup capabilities exposed by the ldk-node builtin app. The thing `payment_timer.rs` calls when it needs to create a per-session BOLT12 offer.

### Migrations

- [`migrations/2026-04-16_add_workspace_pause_fields/`](migrations/2026-04-16_add_workspace_pause_fields/) — **M3**. Adds `paused_at`, `resumed_at`, `total_paused_secs`, `last_billing_tick_at` to `workspace_sessions`. Idempotent `up.sql` + matching `down.sql` rollback.
- [`migrations/2026-04-18_add_bolt12_billing_fields/`](migrations/2026-04-18_add_bolt12_billing_fields/) — **M4.3b**. Adds `bolt12_offer_id`, `bolt12_invoice_id`, `paid_through_at` for BOLT12-based per-minute billing.

### Tests

- [`tests/libvirt_provider_stress_test.rs`](tests/libvirt_provider_stress_test.rs) — concurrent pause/resume stress, verifies idempotency under contention.
- [`tests/workspace_m3_billing_test.rs`](tests/workspace_m3_billing_test.rs) — M3 state-machine test: pause/resume/reconcile gives the expected billing total even under crash-recovery from `last_billing_tick_at`.

### Frontend (React 19 + TypeScript)

- [`frontend/PeerWorkspacePage.tsx`](frontend/PeerWorkspacePage.tsx) — **M4.7**. Alice's rental page. Lists VMs available on a chosen peer, shows price + RAM + storage, click-to-rent flow. After Phase A this page also rendered the CC attestation badge (see `phase-a-cloud-cc/frontend/CcAttestationPanel.tsx`).
- [`frontend/useCloudStore.ts`](frontend/useCloudStore.ts) — React Query hooks for the peer-workspace endpoints.
- [`frontend/workspaceApi.ts`](frontend/workspaceApi.ts) — typed API client. This file later grew the polymorphic `CcAttestation` discriminated union for Phase A.

## What's not included

I deliberately skipped `src/workspace/service.rs` (3500 lines), `handlers.rs` (895), and `models.rs` (543). They're in the source tree and I authored the M2-M4-related parts of them, but they're long and heavily co-authored with the base codebase — putting them in a portfolio reads as code-dump rather than highlight. The pieces above are what shows the shape of the work cleanly: the trait boundary, the per-side transport implementations, the migrations, the tests, the timer logic.

## Commits

M2: `22b9a961` (libvirt provider + stress tests).
M3: `c5dc8540` (pause/resume billing state machine + ledger + reconcile), `b4d60e9e` (reactive resume + ledger HTTP + api-store idempotency).
M4: `e391784e` (M4.1+M4.2 cross-node workspace endpoint + client), `8a6f24eb` (M4.4 auto-register cross-node users on first challenge), `d5a1ed8b` (M4.3b BOLT12 offer creation), `7de7d551` (M4.3b Bob-side BOLT12 billing verification), `97a5a662` (M4.3b Alice-side BOLT12 payment timer), `bb311a32` (BOLT12 capabilities in ldk-node builtin), `ac09fb88` (full wire end-to-end), `15d8c81f` (M4.7 Peer Workspace UI), `04beb9d8` + `dfd3fbe8` (M4 cross-node fixes: computer_id mismatch + late-init node_id for l402).
