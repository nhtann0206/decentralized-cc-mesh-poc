//! Reactive resume handler for paused workspace sessions (M3).
//!
//! Subscribes to `BusEvent::AppDomain` events published by the `api-store`
//! builtin app (`event_name = "api-store.credits.balance_increased"`) and
//! drives any of the payer's paused sessions through
//! `paused → resuming → running` whenever their balance becomes sufficient.
//!
//! This is the realtime complement to the 60-second polling in
//! `WorkspaceService::billing_tick`. If the event path breaks or the handler
//! misses an event, the next tick still catches up — so we tolerate fire-
//! and-forget semantics.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::events::{BusEvent, EventBusType, EventHandler};
use crate::state::AppState;

pub struct WorkspaceCreditsHandler {
    state: Arc<AppState>,
}

impl WorkspaceCreditsHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl EventHandler for WorkspaceCreditsHandler {
    fn handles(&self) -> EventBusType {
        EventBusType::AppDomain
    }

    async fn handle(&self, event: &BusEvent) {
        let Some(app_event) = event.as_app_domain() else {
            return;
        };

        // We only care about balance-went-up signals from api-store. Other
        // AppDomain events (wifi, llm sse, etc.) are handled elsewhere.
        if app_event.event_name != "api-store.credits.balance_increased" {
            return;
        }

        let Some(user_id) = app_event.data.get("user_id").and_then(|v| v.as_str()) else {
            warn!(
                target: "node_backend::events::workspace_credits",
                event = %app_event.event_name,
                "balance_increased event missing user_id — ignoring"
            );
            return;
        };

        let svc_guard = self.state.workspace_service.read().await;
        let Some(svc) = svc_guard.as_ref() else {
            debug!(
                target: "node_backend::events::workspace_credits",
                user_id = %user_id,
                "workspace service not initialized yet — skipping reactive resume"
            );
            return;
        };

        match svc.try_resume_paused_sessions(user_id).await {
            Ok(0) => {
                debug!(
                    target: "node_backend::events::workspace_credits",
                    user_id = %user_id,
                    "no paused sessions to resume"
                );
            }
            Ok(n) => {
                tracing::info!(
                    target: "node_backend::events::workspace_credits",
                    user_id = %user_id,
                    resumed = n,
                    "reactively resumed {} paused session(s) after topup",
                    n
                );
            }
            Err(e) => {
                warn!(
                    target: "node_backend::events::workspace_credits",
                    user_id = %user_id,
                    error = %e,
                    "try_resume_paused_sessions failed — next billing_tick will retry"
                );
            }
        }
    }
}
