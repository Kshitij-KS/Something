PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS captures (
  id INTEGER PRIMARY KEY,
  capture_id TEXT NOT NULL,
  clause_ordinal INTEGER NOT NULL CHECK (clause_ordinal >= 0),
  source_app TEXT NOT NULL CHECK (source_app IN ('slack', 'gmail', 'manual')),
  source_ctx TEXT,
  recipient TEXT,
  raw_message TEXT NOT NULL,
  sent_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (capture_id, clause_ordinal)
);

CREATE TABLE IF NOT EXISTS promises (
  id INTEGER PRIMARY KEY,
  capture_id TEXT NOT NULL,
  clause_ordinal INTEGER NOT NULL,
  text TEXT NOT NULL,
  raw_message TEXT NOT NULL,
  source_app TEXT NOT NULL CHECK (source_app IN ('slack', 'gmail', 'manual')),
  source_ctx TEXT,
  recipient TEXT,
  deadline INTEGER,
  deadline_tz TEXT,
  deadline_precision TEXT CHECK (
    deadline_precision IS NULL
    OR deadline_precision IN ('minute', 'hour', 'day', 'eod', 'eow')
  ),
  confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  score INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (
    status IN ('open', 'done', 'dismissed', 'archived', 'review', 'snoozed')
  ),
  ignore_count INTEGER NOT NULL DEFAULT 0 CHECK (ignore_count >= 0),
  snooze_until INTEGER,
  created_at INTEGER NOT NULL,
  resolved_at INTEGER,
  UNIQUE (capture_id, clause_ordinal),
  FOREIGN KEY (capture_id, clause_ordinal)
    REFERENCES captures (capture_id, clause_ordinal)
    ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS triggers (
  id INTEGER PRIMARY KEY,
  promise_id INTEGER NOT NULL REFERENCES promises(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('app_focus', 'app_ctx_focus', 'deadline', 'manual')),
  match_value TEXT NOT NULL,
  priority INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS surface_attempts (
  id INTEGER PRIMARY KEY,
  promise_id INTEGER NOT NULL REFERENCES promises(id) ON DELETE CASCADE,
  lease_token TEXT NOT NULL UNIQUE,
  action_token TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('leased', 'shown', 'acted', 'expired', 'suppressed')),
  shown_at INTEGER,
  acted_at INTEGER,
  action TEXT CHECK (
    action IS NULL OR action IN ('done', 'snooze', 'not_a_promise', 'ignored')
  ),
  local_day TEXT,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_attempts (
  id INTEGER PRIMARY KEY,
  surface_attempt_id INTEGER NOT NULL REFERENCES surface_attempts(id) ON DELETE CASCADE,
  delivered INTEGER NOT NULL DEFAULT 0 CHECK (delivered IN (0, 1)),
  error TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS selector_health (
  site TEXT PRIMARY KEY CHECK (site IN ('gmail', 'slack')),
  last_probe_at INTEGER,
  last_success_at INTEGER,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  last_capture_at INTEGER,
  state TEXT NOT NULL CHECK (state IN ('healthy', 'degraded', 'broken'))
);

CREATE TABLE IF NOT EXISTS blocklist (
  id INTEGER PRIMARY KEY,
  pattern TEXT NOT NULL UNIQUE,
  hits INTEGER NOT NULL DEFAULT 1 CHECK (hits >= 1),
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS connection_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  state TEXT NOT NULL CHECK (
    state IN ('disconnected', 'handshaking', 'connected', 'reconnecting')
  ),
  last_handshake_at INTEGER,
  host_version TEXT,
  extension_id TEXT
);

CREATE TABLE IF NOT EXISTS phase0_rules (
  id INTEGER PRIMARY KEY,
  app_match TEXT NOT NULL,
  reminder_text TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
);

CREATE TABLE IF NOT EXISTS kill_gates (
  id TEXT PRIMARY KEY,
  status TEXT NOT NULL CHECK (status IN ('pending_user', 'passed', 'failed')),
  notes TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promises_status ON promises(status);
CREATE INDEX IF NOT EXISTS idx_triggers_match ON triggers(kind, match_value);
CREATE INDEX IF NOT EXISTS idx_surface_attempts_promise ON surface_attempts(promise_id, state);
CREATE INDEX IF NOT EXISTS idx_captures_source ON captures(source_app, source_ctx);

INSERT OR IGNORE INTO connection_state (id, state) VALUES (1, 'disconnected');
INSERT OR IGNORE INTO selector_health (site, consecutive_failures, state)
  VALUES ('gmail', 0, 'healthy'), ('slack', 0, 'healthy');
INSERT OR IGNORE INTO kill_gates (id, status, notes) VALUES
  ('phase0_five_day', 'pending_user', 'Use Phase 0 for five days. Stop if context reminders are not materially better than time reminders.'),
  ('extraction_precision_300', 'pending_user', 'Label 300 real sent messages. Require at least 70% precision before Phase 2 and target at least 80% before release.'),
  ('acceptance_two_week', 'pending_user', 'Use the closed loop daily for two weeks. Require at least 40% actionable acceptance before launch work.');
INSERT OR IGNORE INTO settings (k, v) VALUES
  ('daily_surface_cap', '3'),
  ('min_gap_minutes', '90'),
  ('quiet_hours_enabled', 'false'),
  ('quiet_hours_start', '22:00'),
  ('quiet_hours_end', '08:00'),
  ('gmail_enabled', 'true'),
  ('slack_enabled', 'true'),
  ('autostart_enabled', 'false'),
  ('retention_days', '365'),
  ('keyword_app_map', '{"push":"Code.exe","fix":"Code.exe"}');
