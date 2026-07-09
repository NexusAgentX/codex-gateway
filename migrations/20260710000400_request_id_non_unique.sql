CREATE TABLE request_logs_new (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    api_key_id TEXT NOT NULL REFERENCES api_keys(id) ON DELETE RESTRICT,
    model_id TEXT REFERENCES models(id) ON DELETE SET NULL,
    upstream_id TEXT REFERENCES upstreams(id) ON DELETE SET NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status_code INTEGER,
    error_code TEXT,
    stream INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    usage_source TEXT NOT NULL CHECK (usage_source IN ('upstream', 'estimated', 'unknown')) DEFAULT 'unknown',
    input_chars INTEGER NOT NULL DEFAULT 0,
    output_chars INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    client_ip_hash TEXT,
    user_agent TEXT,
    upstream_response_id TEXT,
    upstream_status TEXT,
    client_metadata_sanitized TEXT,
    route_strategy TEXT,
    route_decision_json TEXT
);

INSERT INTO request_logs_new (
    id, request_id, user_id, api_key_id, model_id, upstream_id, method, path, status_code,
    error_code, stream, prompt_tokens, completion_tokens, total_tokens, usage_source, input_chars,
    output_chars, latency_ms, started_at, finished_at, client_ip_hash, user_agent,
    upstream_response_id, upstream_status, client_metadata_sanitized, route_strategy,
    route_decision_json
)
SELECT
    id, request_id, user_id, api_key_id, model_id, upstream_id, method, path, status_code,
    error_code, stream, prompt_tokens, completion_tokens, total_tokens, usage_source, input_chars,
    output_chars, latency_ms, started_at, finished_at, client_ip_hash, user_agent,
    upstream_response_id, upstream_status, client_metadata_sanitized, route_strategy,
    route_decision_json
FROM request_logs;

DROP TABLE request_logs;
ALTER TABLE request_logs_new RENAME TO request_logs;

CREATE INDEX IF NOT EXISTS idx_request_logs_user_started ON request_logs(user_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_key_started ON request_logs(api_key_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_model_started ON request_logs(model_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_upstream_started ON request_logs(upstream_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_status_started ON request_logs(status_code, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_started ON request_logs(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_request_id ON request_logs(request_id, started_at DESC);
