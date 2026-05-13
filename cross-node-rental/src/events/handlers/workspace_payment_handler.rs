//! M4: Update workspace session last_payment_at when a BOLT12 payment
//! is received. Subscribes to ldk-node.payment.received events from the
//! LDK builtin app. When Bob receives a per-minute payment from Alice,
//! this handler finds matching sessions (by bolt12_offer presence + running
//! status) and stamps last_payment_at = now.
//!
//! For Phase 1 with a single tenant per offer, matching is simple: any
//! running session with a non-NULL bolt12_offer gets updated. Phase 2
//! will need payment_hash → session_id mapping via the offer's metadata.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::events::{BusEvent, EventBusType, EventHandler};
use crate::state::AppState;

pub struct WorkspacePaymentHandler {
    state: Arc<AppState>,
}

impl WorkspacePaymentHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl EventHandler for WorkspacePaymentHandler {
    fn handles(&self) -> EventBusType {
        EventBusType::AppDomain
    }

    async fn handle(&self, event: &BusEvent) {
        let Some(app_event) = event.as_app_domain() else {
            return;
        };

        if app_event.event_name != "ldk-node.payment.received" {
            return;
        }

        let amount_msat = app_event
            .data
            .get("amount_msat")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if amount_msat == 0 {
            return;
        }

        // Update last_payment_at for all running BOLT12 sessions.
        // Phase 1 simplification: Bob has few concurrent sessions, so
        // updating all BOLT12 sessions on ANY payment is acceptable.
        // Phase 2: match payment to specific session via offer metadata.
        let svc_guard = self.state.workspace_service.read().await;
        let Some(svc) = svc_guard.as_ref() else {
            return;
        };

        match svc.update_bolt12_payment_received().await {
            Ok(n) if n > 0 => {
                info!(
                    target: "node_backend::events::workspace_payment",
                    amount_msat = amount_msat,
                    sessions_updated = n,
                    "BOLT12 payment received — updated last_payment_at"
                );
            }
            Ok(_) => {
                debug!(
                    target: "node_backend::events::workspace_payment",
                    amount_msat = amount_msat,
                    "Payment received but no BOLT12 sessions to update"
                );
            }
            Err(e) => {
                warn!(
                    target: "node_backend::events::workspace_payment",
                    error = %e,
                    "Failed to update last_payment_at"
                );
            }
        }
    }
}
