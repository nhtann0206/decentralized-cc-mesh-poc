DROP INDEX IF EXISTS idx_workspace_billing_ledger_session;
DROP TABLE IF EXISTS workspace_billing_ledger;

DROP INDEX IF EXISTS idx_workspace_sessions_status_grace;

ALTER TABLE workspace_sessions DROP COLUMN state_version;
ALTER TABLE workspace_sessions DROP COLUMN stop_reason;
ALTER TABLE workspace_sessions DROP COLUMN total_non_billable_secs;
ALTER TABLE workspace_sessions DROP COLUMN non_billable_window_start;
ALTER TABLE workspace_sessions DROP COLUMN grace_expires_at;
ALTER TABLE workspace_sessions DROP COLUMN paused_at;
