# Cross-Node VM Rental

## What this is

The marketplace mechanics of one node renting a VM from another: how the operator spawns the VM, how the renter pays per minute over Lightning, how the session survives pauses and crashes, how an unknown renter is authenticated on first contact, and what the renter sees in the UI. This is the "marketplace" half of the product; Phase A then layers hardware-rooted attestation on top of it.

## What it does

### Operator side: VM lifecycle

- A KVM/libvirt provider that creates, pauses, resumes, and destroys tenant VMs. Every operation is idempotent — concurrent pause-while-resuming, double-create, destroy-during-create are all safe. Stress-tested under contention.
- The provider is wired behind a trait, which is what later let Phase A swap in a GCP-cloud provider on the same call sites without touching anything downstream.

### Billing: pause/resume + reconcile

- A state machine that lets a session pause (operator releases resources, renter stops being billed) and resume cleanly. Survives crashes — on restart the system reconciles the billing total against persisted state instead of double-charging or losing minutes.
- Two database migrations capture the schema changes that make this safe: pause/resume fields, and BOLT12 billing fields. Both ship with matching `down.sql` rollback scripts.

### Lightning: per-minute settlement via BOLT12

- The renter's *payment timer* watches the active session, creates BOLT12 payment ~30s before each billing tick expires, retries with backoff, and triggers a graceful pause if three consecutive payments fail. The operator independently *verifies* each BOLT12 invoice landed before extending the session — neither side trusts the other.
- BOLT12 offer creation and lookup is exposed as a set of capabilities on the LDK builtin app, so the rental flow doesn't talk to LDK internals directly.

### Cross-node identity

- A renter arriving from another node is auto-registered on first authenticated challenge. No prior account; the L402 macaroon flow doubles as the onboarding. The Lightning auth headers carry a late-initialised `node_id` for the cross-node case so the renter is identifiable without leaking secrets.

### Frontend

- The peer-workspace page is the renter's view of "what can I rent on this peer." Lists VMs, prices, RAM, storage, click-to-rent. After Phase A, this page also hosts the CC attestation badge.

## Why it's not just CRUD

The pieces above are easy to get wrong individually and very hard to get right together. Idempotency *and* concurrency *and* crash-recovery *and* per-minute billing *and* cross-node auth — every one of those is a separate bug surface, and the cost of a bug is "real money flows in the wrong direction." Most of the engineering effort in this slice is in making each operation either obviously correct or obviously not happening, never partially.

## Key files

- [`src/workspace/libvirt_provider.rs`](src/workspace/libvirt_provider.rs) — the KVM provider with the idempotency guarantees.
- [`src/workspace/payment_timer.rs`](src/workspace/payment_timer.rs) — the renter's per-minute BOLT12 timer.
- [`src/endpoints/workspace_endpoint.rs`](src/endpoints/workspace_endpoint.rs) — the protocol-agnostic trait both sides of the connection implement.
- [`src/transport/node_client/http_workspace_client.rs`](src/transport/node_client/http_workspace_client.rs) and [`src/transport/node_server/http_workspace_external.rs`](src/transport/node_server/http_workspace_external.rs) — the cross-node HTTP transports.
- [`src/events/handlers/`](src/events/handlers/) — the event-bus reactors that connect LDK payment confirmations to the workspace billing state machine, and the bridge that routes the recurring billing tick through the cron builtin app instead of an ad-hoc tokio interval.
- [`migrations/`](migrations/) — the two schema changes with rollback scripts.
- [`tests/`](tests/) — concurrency stress on the libvirt provider, and a crash-recovery test for the billing reconcile path.
- [`frontend/`](frontend/) — the renter's React UI.

## What's not in this folder

I deliberately left out `service.rs`, `handlers.rs`, and `models.rs` from the workspace module. They exist in the source tree and I authored the M2-through-M4 portions of them, but they're long and heavily co-authored with the surrounding codebase. Putting them here would read as a code dump rather than a highlight. The files above are what shows the *shape* of the work cleanly: the trait boundary, the per-side transports, the migrations, the tests, the timer.
