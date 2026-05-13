-- M4: BOLT12 per-minute billing fields
-- Bob stores the offer string so Alice can pay it repeatedly.
-- last_payment_at tracks when the most recent BOLT12 payment was received,
-- used by billing_tick to detect "Alice stopped paying" → pause.
ALTER TABLE workspace_sessions ADD COLUMN bolt12_offer TEXT;
ALTER TABLE workspace_sessions ADD COLUMN last_payment_at TIMESTAMP;
