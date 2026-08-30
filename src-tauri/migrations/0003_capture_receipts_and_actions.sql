CREATE TABLE IF NOT EXISTS capture_receipts (
  capture_id TEXT PRIMARY KEY,
  payload_sha256 TEXT NOT NULL CHECK (
    length(payload_sha256) = 64
    AND payload_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  source_app TEXT NOT NULL CHECK (source_app IN ('slack', 'gmail', 'manual')),
  sent_at INTEGER NOT NULL,
  timezone TEXT NOT NULL,
  stored_clauses INTEGER NOT NULL DEFAULT 0 CHECK (stored_clauses >= 0),
  committed_at INTEGER NOT NULL
);

DELETE FROM triggers
WHERE id NOT IN (
  SELECT MIN(id)
  FROM triggers
  GROUP BY promise_id, kind, match_value
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_triggers_unique_link
  ON triggers(promise_id, kind, match_value);

ALTER TABLE promises ADD COLUMN deadline_escalated_at INTEGER;
CREATE INDEX IF NOT EXISTS idx_promises_deadline
  ON promises(status, deadline, deadline_escalated_at);
