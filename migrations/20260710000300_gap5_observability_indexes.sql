CREATE INDEX IF NOT EXISTS idx_request_logs_started ON request_logs(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_daily_usage_date ON daily_usage(date DESC);
