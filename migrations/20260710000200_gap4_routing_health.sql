ALTER TABLE upstreams ADD COLUMN health_status_changed_at TEXT;
ALTER TABLE upstreams ADD COLUMN last_degraded_at TEXT;
ALTER TABLE upstreams ADD COLUMN last_down_at TEXT;
ALTER TABLE upstreams ADD COLUMN recent_error_samples TEXT NOT NULL DEFAULT '[]';

ALTER TABLE request_logs ADD COLUMN route_strategy TEXT;
ALTER TABLE request_logs ADD COLUMN route_decision_json TEXT;
