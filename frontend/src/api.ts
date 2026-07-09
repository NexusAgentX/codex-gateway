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
  key: ApiKeySummary;
  plaintext: string;
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
  created_at: string;
  updated_at: string;
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

export class ApiClientError extends Error {
  status: number;
  code: string;

  constructor(status: number, message: string, code: string) {
    super(message);
    this.name = "ApiClientError";
    this.status = status;
    this.code = code;
  }
}

type ApiOptions = {
  method?: string;
  token?: string;
  body?: unknown;
};

export async function apiFetch<T>(path: string, options: ApiOptions = {}): Promise<T> {
  const headers = new Headers();
  if (options.token) {
    headers.set("Authorization", `Bearer ${options.token}`);
  }
  if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, {
    method: options.method ?? "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body)
  });

  if (!response.ok) {
    let message = response.statusText || "Request failed";
    let code = "request_failed";
    try {
      const payload = (await response.json()) as {
        error?: { message?: string; code?: string };
      };
      message = payload.error?.message ?? message;
      code = payload.error?.code ?? code;
    } catch {
      // Non-JSON failures can still be rendered with the HTTP status.
    }
    throw new ApiClientError(response.status, message, code);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}
