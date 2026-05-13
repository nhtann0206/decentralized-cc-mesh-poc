//! CronCommand → `core.cron.*` bridge.
//!
//! Subscribes to `BusEvent::CronCommand` on the event bus and forwards each
//! command to the matching capability on the `cron` builtin app via
//! `CapabilityRouter::invoke`. Before this bridge, publishers like ACME
//! cert renewal, AI agent scheduling, and background task init published
//! `BusEvent::CronCommand` events that had no subscriber after the
//! `404-cron-app-migration` refactor deleted the in-backend `CronService`.
//! This bridge closes the resulting dead-letter gap.
//!
//! Variant routing:
//! - `CreateJob`             → `core.cron.register`
//! - `DeleteByExternalRef`   → `core.cron.unregister`
//! - `PauseJob`              → `core.cron.pause`
//! - `ResumeJob`             → `core.cron.resume`
//! - `UpdateJob` / `DeleteJob` → no-op + warn (the cron builtin's public API
//!   is keyed on `external_type + external_id`, so callers should use
//!   `DeleteByExternalRef` + `CreateJob` to express updates and targeted
//!   deletes. Logging a warning surfaces any stray publisher that still
//!   uses the obsolete variants.)

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::events::event_types::CronCommand;
use crate::events::{BusEvent, EventBusType, EventHandler};
use crate::state::AppState;

const BRIDGE_CALLER: &str = "cron_command_bridge";
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CronCommandBridge {
    state: Arc<AppState>,
}

impl CronCommandBridge {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Call a capability on the cron builtin app through the router.
    /// Returns Err on any failure (router not yet initialized, builtin not
    /// loaded, capability not found, timeout, or remote error). The caller
    /// is expected to log + continue — the event-bus handler contract does
    /// not allow propagating errors back to the bus.
    async fn invoke_cron(&self, capability: &str, payload: Value) -> Result<Value, String> {
        let router_guard = self.state.capability_router.read().await;
        let Some(router) = router_guard.as_ref().cloned() else {
            return Err("capability router not yet initialized".to_string());
        };
        drop(router_guard);

        router
            .invoke(
                BRIDGE_CALLER,
                capability,
                payload,
                Some(BRIDGE_TIMEOUT),
                None,
            )
            .await
            .map_err(|e| format!("{:?}", e))
    }

    /// Translate a `CronCommand::CreateJob` into the JSON body that
    /// `core.cron.register` (`CapabilityRegisterRequest` in the builtin
    /// app) expects.
    ///
    /// `handler_id` carries the trigger kind by convention from
    /// `CreateCronJobBuilder`:
    /// - `"capability"` → capability trigger, `config` must contain
    ///   `{ capability: <name>, payload: <value> }`
    /// - anything else (`"internal_api"` is the default) → HTTP trigger,
    ///   `config` carries `{ method, endpoint, body?, headers? }`
    fn build_register_payload(cmd: &CronCommand) -> Option<Value> {
        let CronCommand::CreateJob {
            external_type,
            external_id,
            name,
            job_type,
            handler_id,
            schedule_expression,
            timezone,
            config,
            max_retries,
            retry_delay_ms,
            failure_threshold,
            max_instances: _drop,
        } = cmd
        else {
            return None;
        };

        let trigger = if handler_id == "capability" {
            let capability_name = config
                .get("capability")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let payload = config.get("payload").cloned().unwrap_or_else(|| json!({}));
            json!({
                "type": "capability",
                "capability": capability_name,
                "payload": payload,
            })
        } else {
            let method = config
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_string();
            let endpoint = config
                .get("endpoint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let body = config.get("body").cloned();
            let headers = config.get("headers").cloned();
            json!({
                "type": "http",
                "method": method,
                "endpoint": endpoint,
                "body": body,
                "headers": headers,
            })
        };

        Some(json!({
            "name": name,
            "schedule": schedule_expression,
            "timezone": timezone,
            "job_type": job_type.as_str(),
            "trigger": trigger,
            "external_type": external_type.as_str(),
            "external_id": external_id,
            "max_retries": max_retries,
            "retry_delay_ms": retry_delay_ms,
            "failure_threshold": failure_threshold,
        }))
    }
}

#[async_trait]
impl EventHandler for CronCommandBridge {
    fn handles(&self) -> EventBusType {
        EventBusType::CronCommand
    }

    async fn handle(&self, event: &BusEvent) {
        let Some(cmd) = event.as_cron_command() else {
            return;
        };

        debug!(
            target: "node_backend::events::cron_bridge",
            command = ?cmd,
            "routing CronCommand to cron builtin app"
        );

        let (capability, payload, context_key) = match cmd {
            CronCommand::CreateJob {
                external_type,
                external_id,
                ..
            } => {
                let Some(payload) = Self::build_register_payload(cmd) else {
                    return;
                };
                (
                    "core.cron.register",
                    payload,
                    format!("{}:{}", external_type.as_str(), external_id),
                )
            }
            CronCommand::DeleteByExternalRef {
                external_type,
                external_id,
            } => (
                "core.cron.unregister",
                json!({
                    "external_type": external_type.as_str(),
                    "external_id": external_id,
                }),
                format!("{}:{}", external_type.as_str(), external_id),
            ),
            CronCommand::PauseJob { job_id } => (
                "core.cron.pause",
                json!({ "job_id": job_id }),
                job_id.clone(),
            ),
            CronCommand::ResumeJob { job_id } => (
                "core.cron.resume",
                json!({ "job_id": job_id }),
                job_id.clone(),
            ),
            CronCommand::UpdateJob { job_id, .. } => {
                warn!(
                    target: "node_backend::events::cron_bridge",
                    job_id = %job_id,
                    "CronCommand::UpdateJob is not supported by the cron builtin app; \
                     publishers should use DeleteByExternalRef + CreateJob to express updates"
                );
                return;
            }
            CronCommand::DeleteJob { job_id } => {
                warn!(
                    target: "node_backend::events::cron_bridge",
                    job_id = %job_id,
                    "CronCommand::DeleteJob is not supported by the cron builtin app; \
                     publishers should use DeleteByExternalRef keyed on external_type+external_id"
                );
                return;
            }
        };

        match self.invoke_cron(capability, payload).await {
            Ok(_) => {
                info!(
                    target: "node_backend::events::cron_bridge",
                    capability = %capability,
                    context = %context_key,
                    "forwarded CronCommand to cron builtin app"
                );
            }
            Err(e) => {
                error!(
                    target: "node_backend::events::cron_bridge",
                    capability = %capability,
                    context = %context_key,
                    error = %e,
                    "failed to forward CronCommand to cron builtin app"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::cron_types::{CronExternalType, CronJobType};

    #[test]
    fn build_register_payload_http_trigger() {
        let cmd = CronCommand::CreateJob {
            external_type: CronExternalType::Certificate,
            external_id: "cert-xyz".into(),
            name: "Certificate Renewal: 1.2.3.4".into(),
            job_type: CronJobType::Sync,
            handler_id: "internal_api".into(),
            schedule_expression: "0 0 2 * * *".into(),
            timezone: Some("UTC".into()),
            config: json!({
                "method": "POST",
                "endpoint": "/api/v2/internal/certificates/cert-xyz/renew",
                "body": { "certificate_id": "cert-xyz" },
                "headers": { "X-Internal-Service": "cron-scheduler" },
            }),
            max_instances: Some(1),
            max_retries: Some(3),
            retry_delay_ms: Some(5000),
            failure_threshold: Some(5),
        };

        let payload = CronCommandBridge::build_register_payload(&cmd).expect("payload built");
        assert_eq!(payload["name"], "Certificate Renewal: 1.2.3.4");
        assert_eq!(payload["schedule"], "0 0 2 * * *");
        assert_eq!(payload["external_type"], "certificate");
        assert_eq!(payload["external_id"], "cert-xyz");
        assert_eq!(payload["trigger"]["type"], "http");
        assert_eq!(payload["trigger"]["method"], "POST");
        assert_eq!(
            payload["trigger"]["endpoint"],
            "/api/v2/internal/certificates/cert-xyz/renew"
        );
        assert_eq!(
            payload["trigger"]["body"]["certificate_id"],
            "cert-xyz"
        );
        assert_eq!(payload["max_retries"], 3);
        // max_instances is intentionally dropped (cron builtin does not expose it).
        assert!(payload.get("max_instances").is_none());
    }

    #[test]
    fn build_register_payload_capability_trigger() {
        let cmd = CronCommand::CreateJob {
            external_type: CronExternalType::System,
            external_id: "cleanup-temp".into(),
            name: "Storage Cleanup".into(),
            job_type: CronJobType::Cleanup,
            handler_id: "capability".into(),
            schedule_expression: "0 0 3 * * *".into(),
            timezone: None,
            config: json!({
                "capability": "core.storage.cleanup",
                "payload": { "collection": "temp", "max_age_days": 30 },
            }),
            max_instances: Some(1),
            max_retries: Some(0),
            retry_delay_ms: Some(0),
            failure_threshold: Some(1),
        };

        let payload = CronCommandBridge::build_register_payload(&cmd).expect("payload built");
        assert_eq!(payload["trigger"]["type"], "capability");
        assert_eq!(payload["trigger"]["capability"], "core.storage.cleanup");
        assert_eq!(payload["trigger"]["payload"]["collection"], "temp");
        assert_eq!(payload["trigger"]["payload"]["max_age_days"], 30);
    }

    #[test]
    fn build_register_payload_defaults_method_when_missing() {
        let cmd = CronCommand::CreateJob {
            external_type: CronExternalType::Custom,
            external_id: "custom-1".into(),
            name: "Custom Job".into(),
            job_type: CronJobType::Custom,
            handler_id: "internal_api".into(),
            schedule_expression: "*/30 * * * * *".into(),
            timezone: None,
            config: json!({ "endpoint": "/api/v2/custom" }),
            max_instances: None,
            max_retries: None,
            retry_delay_ms: None,
            failure_threshold: None,
        };

        let payload = CronCommandBridge::build_register_payload(&cmd).expect("payload built");
        assert_eq!(payload["trigger"]["method"], "GET");
        assert_eq!(payload["trigger"]["endpoint"], "/api/v2/custom");
    }
}
