export type LoginUser = {
  id: string;
  email: string;
  role: string;
};

export type AuthenticatedUser = {
  user_id: string;
  api_key_id: string;
  key_prefix: string;
  email: string;
  role: string;
};

export type ApiKeySummary = {
  id: string;
  user_id: string;
  name: string;
  key_prefix: string;
  status: string;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
  revoked_at: string | null;
};

export type LoginResponse = {
  user: LoginUser;
  token: string;
  token_type: string;
};

export type RequestLog = {
  id: string;
  request_id: string;
  user_id: string;
  api_key_id: string;
  model_id: string | null;
  upstream_id: string | null;
  method: string;
  path: string;
  status_code: number | null;
  error_code: string | null;
  stream: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  usage_source: string;
  input_chars: number;
  output_chars: number;
  latency_ms: number;
  started_at: string;
  finished_at: string | null;
  upstream_response_id: string | null;
  upstream_status: string | null;
  client_metadata_sanitized: string | null;
  route_strategy: string | null;
  route_decision_json: string | null;
};

export type DailyUsage = {
  date: string;
  user_id: string;
  api_key_id: string;
  model_id: string | null;
  upstream_id: string | null;
  request_count: number;
  error_count: number;
  stream_count: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  latency_ms_sum: number;
};

export type OverviewResponse = {
  user: AuthenticatedUser;
  daily_usage: DailyUsage[];
  recent_requests: RequestLog[];
};

export type User = {
  id: string;
  email: string;
  role: string;
  status: string;
  display_name: string | null;
  created_at: string;
  updated_at: string;
  last_login_at: string | null;
};

export type Upstream = {
  id: string;
  name: string;
  base_url: string;
  enabled: number;
  priority: number;
  weight: number;
  timeout_ms: number;
  max_retries: number;
  health_check_path: string;
  last_health_status: string;
  last_health_checked_at: string | null;
  health_status_changed_at: string | null;
  last_degraded_at: string | null;
  last_down_at: string | null;
  recent_error_samples: string;
  created_at: string;
  updated_at: string;
};

export type GatewayMetrics = {
  generated_at: string;
  request_count: number;
  error_count: number;
  latency: {
    sum_ms: number;
    avg_ms: number | null;
  };
  token_usage: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
  upstream_health: Array<{
    upstream_id: string;
    name: string;
    enabled: number;
    last_health_status: string;
    last_health_checked_at: string | null;
    last_degraded_at: string | null;
    last_down_at: string | null;
    recent_error_samples: string;
    request_count: number;
    error_count: number;
    latency_ms_sum: number;
    total_tokens: number;
  }>;
};

export type Model = {
  id: string;
  public_name: string;
  description: string | null;
  enabled: number;
  visible_to_users: number;
  created_at: string;
  updated_at: string;
};

export type ModelMapping = {
  id: string;
  model_id: string;
  upstream_id: string;
  upstream_model_name: string;
  enabled: number;
  priority: number;
  weight: number;
  created_at: string;
  updated_at: string;
};

export type SettingsSummary = {
  service: string;
  public_url: string;
  bind: string;
  log_level: string;
  route_strategy: string;
  health_checks_enabled: boolean;
  health_check_interval_ms: number;
  request_log_retention_days: number;
  daily_usage_retention_days: number;
  retention_run_on_startup: boolean;
  admin_email_configured: boolean;
  bootstrap_admin_key_configured: boolean;
  database: {
    kind: string;
    configured: boolean;
  };
  counts: {
    users: number;
    api_keys: number;
    upstreams: number;
    models: number;
    request_logs: number;
  };
};

export type LimitPolicy = {
  scope: string;
  subject_id: string;
  request_quota: number | null;
  request_quota_mode: string;
  request_window_seconds: number;
  token_quota: number | null;
  token_quota_mode: string;
  token_window_seconds: number;
  rate_limit_requests: number | null;
  rate_limit_mode: string;
  rate_limit_window_seconds: number;
  concurrency_limit: number | null;
  concurrency_mode: string;
  created_at: string;
  updated_at: string;
};

export type LimitBucketState = {
  limit: number | null;
  used: number;
  remaining: number | null;
  window_seconds: number | null;
  reset_at: string | null;
};

export type LimitSubjectState = {
  scope: string;
  subject_id: string;
  policy: LimitPolicy;
  effective_policy: LimitPolicy;
  request_quota: LimitBucketState;
  token_budget: LimitBucketState;
  rate_limit: LimitBucketState;
  concurrency: {
    limit: number | null;
    in_flight: number;
    remaining: number | null;
  };
};

export type UserLimitState = {
  user: LimitSubjectState;
  current_key: LimitSubjectState | null;
  api_keys: LimitSubjectState[];
};

export type AdminLimitState = {
  system: LimitPolicy;
  users: LimitSubjectState[];
  api_keys: LimitSubjectState[];
};
