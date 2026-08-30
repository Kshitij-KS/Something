ALTER TABLE selector_health ADD COLUMN first_observed_at INTEGER;

UPDATE selector_health
SET first_observed_at = COALESCE(first_observed_at, last_probe_at, last_capture_at);
