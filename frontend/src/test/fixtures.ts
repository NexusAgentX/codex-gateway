import type { AdminLimitState, LimitPolicy, SettingsSummary, Upstream } from "../types/api";

export const limitPolicy: LimitPolicy = {
  scope: "system",
  subject_id: "",
  request_quota: null,
  request_quota_mode: "unlimited",
  request_window_seconds: 86400,
  token_quota: null,
  token_quota_mode: "unlimited",
  token_window_seconds: 86400,
  rate_limit_requests: null,
  rate_limit_mode: "unlimited",
  rate_limit_window_seconds: 60,
  concurrency_limit: null,
  concurrency_mode: "unlimited",
  created_at: "2026-07-12T00:00:00Z",
  updated_at: "2026-07-12T00:00:00Z"
};

export const adminLimits: AdminLimitState = {
  system: limitPolicy,
  users: [],
  api_keys: []
};

export const settings: SettingsSummary = {
  service: "codex-gateway",
  public_url: "http://localhost",
  bind: "127.0.0.1:8080",
  log_level: "info",
  route_strategy: "priority",
  default_request_timeout_ms: 120000,
  max_request_body_bytes: 1048576,
  health_checks_enabled: true,
  health_check_interval_ms: 30000,
  request_log_retention_days: 90,
  daily_usage_retention_days: 730,
  retention_run_on_startup: true,
  expose_debug_headers: false,
  admin_email_configured: false,
  bootstrap_admin_key_configured: false,
  database: {
    kind: "sqlite",
    configured: true,
    settings: {
      route_strategy: null,
      default_request_timeout_ms: null,
      max_request_body_bytes: null,
      request_log_retention_days: null,
      daily_usage_retention_days: null,
      expose_debug_headers: null,
      updated_at: "2026-07-12T00:00:00Z"
    }
  },
  runtime: { precedence: "environment > database > default", fields: [] },
  environment: [],
  default_limit_policy: limitPolicy,
  counts: { users: 1, api_keys: 0, upstreams: 0, models: 1, request_logs: 0 }
};

export const upstream: Upstream = {
  id: "upstream-1",
  name: "Primary",
  base_url: "https://upstream.example",
  enabled: true,
  priority: 1,
  weight: 1,
  timeout_ms: 120000,
  timeout_ms_is_explicit: false,
  max_retries: 1,
  health_check_path: "/v1/models",
  last_health_status: "healthy",
  last_health_checked_at: "2026-07-12T00:00:00Z",
  health_status_changed_at: "2026-07-12T00:00:00Z",
  last_degraded_at: null,
  last_down_at: null,
  recent_error_samples: "[]",
  created_at: "2026-07-12T00:00:00Z",
  updated_at: "2026-07-12T00:00:00Z"
};
