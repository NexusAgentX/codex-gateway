CREATE TABLE IF NOT EXISTS limit_policies (
    scope TEXT NOT NULL CHECK (scope IN ('system', 'user', 'api_key')),
    subject_id TEXT NOT NULL DEFAULT '',
    request_quota INTEGER CHECK (request_quota IS NULL OR request_quota >= 0),
    request_quota_mode TEXT NOT NULL CHECK (request_quota_mode IN ('inherit', 'limited', 'unlimited')) DEFAULT 'inherit',
    request_window_seconds INTEGER NOT NULL DEFAULT 86400 CHECK (request_window_seconds > 0),
    token_quota INTEGER CHECK (token_quota IS NULL OR token_quota >= 0),
    token_quota_mode TEXT NOT NULL CHECK (token_quota_mode IN ('inherit', 'limited', 'unlimited')) DEFAULT 'inherit',
    token_window_seconds INTEGER NOT NULL DEFAULT 86400 CHECK (token_window_seconds > 0),
    rate_limit_requests INTEGER CHECK (rate_limit_requests IS NULL OR rate_limit_requests >= 0),
    rate_limit_mode TEXT NOT NULL CHECK (rate_limit_mode IN ('inherit', 'limited', 'unlimited')) DEFAULT 'inherit',
    rate_limit_window_seconds INTEGER NOT NULL DEFAULT 60 CHECK (rate_limit_window_seconds > 0),
    concurrency_limit INTEGER CHECK (concurrency_limit IS NULL OR concurrency_limit >= 0),
    concurrency_mode TEXT NOT NULL CHECK (concurrency_mode IN ('inherit', 'limited', 'unlimited')) DEFAULT 'inherit',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (scope, subject_id)
);

INSERT OR IGNORE INTO limit_policies (
    scope,
    subject_id,
    request_quota,
    request_quota_mode,
    request_window_seconds,
    token_quota,
    token_quota_mode,
    token_window_seconds,
    rate_limit_requests,
    rate_limit_mode,
    rate_limit_window_seconds,
    concurrency_limit,
    concurrency_mode
) VALUES ('system', '', NULL, 'unlimited', 86400, NULL, 'unlimited', 86400, NULL, 'unlimited', 60, NULL, 'unlimited');

CREATE TABLE IF NOT EXISTS limit_usage_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 1,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    finalized_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_limit_usage_user_created ON limit_usage_events(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_limit_usage_key_created ON limit_usage_events(api_key_id, created_at DESC);

CREATE TABLE IF NOT EXISTS limit_rate_counters (
    scope TEXT NOT NULL CHECK (scope IN ('user', 'api_key')),
    subject_id TEXT NOT NULL,
    window_started_at TEXT NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (scope, subject_id, window_started_at)
);

CREATE TABLE IF NOT EXISTS limit_inflight_requests (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    api_key_id TEXT NOT NULL,
    started_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_limit_inflight_user ON limit_inflight_requests(user_id, expires_at);
CREATE INDEX IF NOT EXISTS idx_limit_inflight_key ON limit_inflight_requests(api_key_id, expires_at);
