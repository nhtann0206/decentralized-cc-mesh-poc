-- M3: pause/resume billing semantics.
--
-- New columns on workspace_sessions for the paused/resuming state machine:
--   paused_at: when the session entered the `paused` state
--   grace_expires_at: deadline after which no-topup = stop
--   non_billable_window_start: set on running→pausing/stopping transitions,
--     cleared on resuming→running. Used to compute time spent out of billable
--     state without mutating billable_started_at
--   total_non_billable_secs: accumulator of completed non-billable windows
--     (pausing + paused + resuming + prior stopping); bigint for defensive
--     overflow margin
--   stop_reason: snake_case string of StopReason enum; NULL when running
--   state_version: CAS token bumped on every status change so concurrent
--     UPDATE ... WHERE status=? AND state_version=? can detect lost race
ALTER TABLE workspace_sessions ADD COLUMN paused_at TIMESTAMP;
ALTER TABLE workspace_sessions ADD COLUMN grace_expires_at TIMESTAMP;
ALTER TABLE workspace_sessions ADD COLUMN non_billable_window_start TIMESTAMP;
ALTER TABLE workspace_sessions ADD COLUMN total_non_billable_secs BIGINT NOT NULL DEFAULT 0;
ALTER TABLE workspace_sessions ADD COLUMN stop_reason TEXT;
ALTER TABLE workspace_sessions ADD COLUMN state_version INTEGER NOT NULL DEFAULT 0;

-- Billing tick needs fast lookup of paused sessions whose grace window elapsed
CREATE INDEX IF NOT EXISTS idx_workspace_sessions_status_grace
    ON workspace_sessions(status, grace_expires_at);

-- Append-only audit ledger for per-session billing events. Every tick charge,
-- deposit, refund, or rollback gets a row. Balance derived from SUM(delta_sats)
-- scoped by session. The UNIQUE(session_id, ref_id) makes ledger inserts
-- idempotent under retry: billing_tick passes ref_id = "tick:{session}:{minute}"
-- and duplicate fires become no-ops via INSERT OR IGNORE.
--
-- Balance source of truth for the Phase 1 demo remains the api-store credits
-- table (accessed via capability router). This ledger is audit-only: renter
-- can read it to verify billing history. A reconciliation job can detect
-- divergence between ledger SUM and api-store balance.
CREATE TABLE IF NOT EXISTS workspace_billing_ledger (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id          TEXT    NOT NULL REFERENCES workspace_sessions(id) ON DELETE RESTRICT,
    minute_seq          INTEGER,
    delta_sats          INTEGER NOT NULL,
    reason              TEXT    NOT NULL,
    ref_id              TEXT    NOT NULL,
    balance_before_sats INTEGER,
    balance_after_sats  INTEGER,
    created_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(session_id, ref_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_billing_ledger_session
    ON workspace_billing_ledger(session_id, created_at DESC);
