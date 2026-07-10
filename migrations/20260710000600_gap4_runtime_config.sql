CREATE TABLE IF NOT EXISTS system_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    route_strategy TEXT CHECK (route_strategy IS NULL OR route_strategy IN ('priority', 'weighted', 'sticky_by_key')),
    default_request_timeout_ms INTEGER CHECK (default_request_timeout_ms IS NULL OR default_request_timeout_ms > 0),
    max_request_body_bytes INTEGER CHECK (max_request_body_bytes IS NULL OR max_request_body_bytes > 0),
    request_log_retention_days INTEGER CHECK (request_log_retention_days IS NULL OR request_log_retention_days >= 0),
    daily_usage_retention_days INTEGER CHECK (daily_usage_retention_days IS NULL OR daily_usage_retention_days >= 0),
    expose_debug_headers INTEGER CHECK (expose_debug_headers IS NULL OR expose_debug_headers IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT OR IGNORE INTO system_config (id) VALUES (1);

ALTER TABLE upstreams ADD COLUMN timeout_ms_is_explicit INTEGER NOT NULL DEFAULT 1 CHECK (timeout_ms_is_explicit IN (0, 1));
