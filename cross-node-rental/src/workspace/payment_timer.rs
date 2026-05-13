//! M4: Alice-side BOLT12 payment timer.
//!
//! When Alice rents a VM from Bob, Alice's backend runs a timer that
//! pays Bob's BOLT12 offer every billing interval (default 60s). This
//! is the client-push model: Alice proves she paid, Bob verifies.
//!
//! The timer starts when Alice receives a session response containing
//! a `bolt12_offer`. It stops when Alice calls stop_session or when
//! the payment fails (insufficient LN balance → Bob will pause VM).
//!
//! Architecture: a background tokio task per active remote session.
//! Tasks are spawned by `start_payment_timer()` and cancelled by
//! `stop_payment_timer()`. The task map is held in `PaymentTimerManager`.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Manages per-session payment timer tasks.
pub struct PaymentTimerManager {
    /// session_id → task handle. Dropping the handle cancels the task.
    timers: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl PaymentTimerManager {
    pub fn new() -> Self {
        Self {
            timers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start a payment timer for a remote session. Spawns a tokio task
    /// that calls `pay_bolt12_offer` via capability router every
    /// `interval_secs` seconds.
    ///
    /// `offer_str`: BOLT12 offer from Bob's session response
    /// `rate_msat`: amount to pay per interval (sats/min * 1000)
    /// `session_id`: for logging + payer_note metadata
    /// `state`: AppState for capability router access
    pub async fn start_timer(
        &self,
        session_id: String,
        offer_str: String,
        rate_msat: u64,
        interval_secs: u64,
        state: Arc<crate::state::AppState>,
    ) {
        let mut timers = self.timers.lock().await;

        // Cancel existing timer for this session (idempotent restart)
        if let Some(old) = timers.remove(&session_id) {
            old.abort();
        }

        let sid = session_id.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(interval_secs),
            );
            // Skip the first tick (fires immediately) — give the session
            // a moment to stabilize before first payment.
            interval.tick().await;

            let mut minute_seq: u64 = 1;
            loop {
                interval.tick().await;

                let payer_note = format!("minute:{},session:{}", minute_seq, &sid[..8.min(sid.len())]);

                let router_guard = state.capability_router.read().await;
                let Some(router) = router_guard.as_ref() else {
                    warn!(
                        target: "node_backend::workspace::payment_timer",
                        session_id = %sid,
                        "CapabilityRouter not available — skipping payment"
                    );
                    continue;
                };

                match router
                    .invoke(
                        "workspace_payment_timer",
                        "core.lightning.pay_bolt12_offer",
                        serde_json::json!({
                            "offer": offer_str,
                            "amount_msat": rate_msat,
                            "payer_note": payer_note,
                        }),
                        None,
                        None,
                    )
                    .await
                {
                    Ok(resp) => {
                        let status = resp
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        if status == "succeeded" {
                            debug!(
                                target: "node_backend::workspace::payment_timer",
                                session_id = %sid,
                                minute_seq = minute_seq,
                                amount_msat = rate_msat,
                                "BOLT12 payment sent"
                            );
                        } else {
                            warn!(
                                target: "node_backend::workspace::payment_timer",
                                session_id = %sid,
                                minute_seq = minute_seq,
                                status = %status,
                                "BOLT12 payment pending/unknown"
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            target: "node_backend::workspace::payment_timer",
                            session_id = %sid,
                            minute_seq = minute_seq,
                            error = %e,
                            "BOLT12 payment failed — Bob will pause VM"
                        );
                        // Don't break — let Bob's billing_tick detect
                        // the missing payment and pause. Alice timer
                        // continues trying in case it was a transient
                        // routing error.
                    }
                }
                minute_seq += 1;
            }
        });

        timers.insert(session_id.clone(), handle);
        info!(
            target: "node_backend::workspace::payment_timer",
            session_id = %session_id,
            interval_secs = interval_secs,
            rate_msat = rate_msat,
            "Payment timer started"
        );
    }

    /// Stop the payment timer for a session (Alice clicked stop or
    /// session ended).
    pub async fn stop_timer(&self, session_id: &str) {
        let mut timers = self.timers.lock().await;
        if let Some(handle) = timers.remove(session_id) {
            handle.abort();
            info!(
                target: "node_backend::workspace::payment_timer",
                session_id = %session_id,
                "Payment timer stopped"
            );
        }
    }

    /// Stop all timers (shutdown).
    pub async fn stop_all(&self) {
        let mut timers = self.timers.lock().await;
        for (sid, handle) in timers.drain() {
            handle.abort();
            debug!(
                target: "node_backend::workspace::payment_timer",
                session_id = %sid,
                "Payment timer stopped (shutdown)"
            );
        }
    }
}
