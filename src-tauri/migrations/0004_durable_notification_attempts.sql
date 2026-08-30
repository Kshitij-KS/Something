DELETE FROM notification_attempts
WHERE id NOT IN (
  SELECT MIN(id)
  FROM notification_attempts
  GROUP BY surface_attempt_id
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_attempt_surface
  ON notification_attempts(surface_attempt_id);

CREATE INDEX IF NOT EXISTS idx_promises_snooze
  ON promises(status, snooze_until);
