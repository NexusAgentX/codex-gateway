use serde::Serialize;

use crate::{auth, storage};

fn response_bool(value: i64) -> Result<bool, super::ApiError> {
    storage::sqlite_bool(value).map_err(Into::into)
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AuthenticatedUserResponse {
    pub user_id: String,
    pub api_key_id: String,
    pub key_prefix: String,
    pub email: String,
    pub role: String,
}

impl From<auth::AuthenticatedUser> for AuthenticatedUserResponse {
    fn from(value: auth::AuthenticatedUser) -> Self {
        Self {
            user_id: value.user_id,
            api_key_id: value.api_key_id,
            key_prefix: value.key_prefix,
            email: value.email,
            role: value.role,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UserResponse {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub display_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

impl From<storage::User> for UserResponse {
    fn from(value: storage::User) -> Self {
        Self {
            id: value.id,
            email: value.email,
            role: value.role,
            status: value.status,
            display_name: value.display_name,
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_login_at: value.last_login_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ApiKeyResponse {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_prefix: String,
    pub status: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl From<storage::ApiKeySummary> for ApiKeyResponse {
    fn from(value: storage::ApiKeySummary) -> Self {
        Self {
            id: value.id,
            user_id: value.user_id,
            name: value.name,
            key_prefix: value.key_prefix,
            status: value.status,
            last_used_at: value.last_used_at,
            expires_at: value.expires_at,
            created_at: value.created_at,
            revoked_at: value.revoked_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UpstreamResponse {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub priority: i64,
    pub weight: i64,
    pub timeout_ms: i64,
    pub timeout_ms_is_explicit: bool,
    pub max_retries: i64,
    pub health_check_path: String,
    pub last_health_status: String,
    pub last_health_checked_at: Option<String>,
    pub health_status_changed_at: Option<String>,
    pub last_degraded_at: Option<String>,
    pub last_down_at: Option<String>,
    pub recent_error_samples: String,
    pub created_at: String,
    pub updated_at: String,
}

impl UpstreamResponse {
    pub fn try_from_record(
        mut value: storage::Upstream,
        default_timeout_ms: i64,
    ) -> Result<Self, super::ApiError> {
        let enabled = response_bool(value.enabled)?;
        let timeout_ms_is_explicit = response_bool(value.timeout_ms_is_explicit)?;
        if !timeout_ms_is_explicit {
            value.timeout_ms = default_timeout_ms;
        }
        Ok(Self {
            id: value.id,
            name: value.name,
            base_url: value.base_url,
            enabled,
            priority: value.priority,
            weight: value.weight,
            timeout_ms: value.timeout_ms,
            timeout_ms_is_explicit,
            max_retries: value.max_retries,
            health_check_path: value.health_check_path,
            last_health_status: value.last_health_status,
            last_health_checked_at: value.last_health_checked_at,
            health_status_changed_at: value.health_status_changed_at,
            last_degraded_at: value.last_degraded_at,
            last_down_at: value.last_down_at,
            recent_error_samples: value.recent_error_samples,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ModelResponse {
    pub id: String,
    pub public_name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub visible_to_users: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl TryFrom<storage::Model> for ModelResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::Model) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            public_name: value.public_name,
            description: value.description,
            enabled: response_bool(value.enabled)?,
            visible_to_users: response_bool(value.visible_to_users)?,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ModelMappingResponse {
    pub id: String,
    pub model_id: String,
    pub upstream_id: String,
    pub upstream_model_name: String,
    pub enabled: bool,
    pub priority: i64,
    pub weight: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl TryFrom<storage::UpstreamModel> for ModelMappingResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::UpstreamModel) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            model_id: value.model_id,
            upstream_id: value.upstream_id,
            upstream_model_name: value.upstream_model_name,
            enabled: response_bool(value.enabled)?,
            priority: value.priority,
            weight: value.weight,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RequestLogResponse {
    pub id: String,
    pub request_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: Option<i64>,
    pub error_code: Option<String>,
    pub stream: bool,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub usage_source: String,
    pub input_chars: i64,
    pub output_chars: i64,
    pub latency_ms: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub client_ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub upstream_response_id: Option<String>,
    pub upstream_status: Option<String>,
    pub client_metadata_sanitized: Option<String>,
    pub route_strategy: Option<String>,
    pub route_decision_json: Option<String>,
}

impl TryFrom<storage::RequestLogRow> for RequestLogResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::RequestLogRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            request_id: value.request_id,
            user_id: value.user_id,
            api_key_id: value.api_key_id,
            model_id: value.model_id,
            upstream_id: value.upstream_id,
            method: value.method,
            path: value.path,
            status_code: value.status_code,
            error_code: value.error_code,
            stream: response_bool(value.stream)?,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            usage_source: value.usage_source,
            input_chars: value.input_chars,
            output_chars: value.output_chars,
            latency_ms: value.latency_ms,
            started_at: value.started_at,
            finished_at: value.finished_at,
            client_ip_hash: value.client_ip_hash,
            user_agent: value.user_agent,
            upstream_response_id: value.upstream_response_id,
            upstream_status: value.upstream_status,
            client_metadata_sanitized: value.client_metadata_sanitized,
            route_strategy: value.route_strategy,
            route_decision_json: value.route_decision_json,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct DailyUsageResponse {
    pub date: String,
    pub user_id: String,
    pub api_key_id: String,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub stream_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
}

impl From<storage::DailyUsageRow> for DailyUsageResponse {
    fn from(value: storage::DailyUsageRow) -> Self {
        Self {
            date: value.date,
            user_id: value.user_id,
            api_key_id: value.api_key_id,
            model_id: value.model_id,
            upstream_id: value.upstream_id,
            request_count: value.request_count,
            error_count: value.error_count,
            stream_count: value.stream_count,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            latency_ms_sum: value.latency_ms_sum,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct OverviewResponse {
    pub user: AuthenticatedUserResponse,
    pub daily_usage: Vec<DailyUsageResponse>,
    pub recent_requests: Vec<RequestLogResponse>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UsageTotalsResponse {
    pub request_count: i64,
    pub error_count: i64,
    pub stream_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
    pub error_rate: f64,
}

impl From<storage::UsageTotals> for UsageTotalsResponse {
    fn from(value: storage::UsageTotals) -> Self {
        Self {
            request_count: value.request_count,
            error_count: value.error_count,
            stream_count: value.stream_count,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            latency_ms_sum: value.latency_ms_sum,
            error_rate: value.error_rate,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ErrorSummaryResponse {
    pub error_code: String,
    pub status_code: Option<i64>,
    pub count: i64,
    pub last_seen_at: Option<String>,
}

impl From<storage::ErrorSummaryRow> for ErrorSummaryResponse {
    fn from(value: storage::ErrorSummaryRow) -> Self {
        Self {
            error_code: value.error_code,
            status_code: value.status_code,
            count: value.count,
            last_seen_at: value.last_seen_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UsageSummaryResponse {
    pub totals: UsageTotalsResponse,
    pub errors: Vec<ErrorSummaryResponse>,
    pub recent_failures: Vec<RequestLogResponse>,
}

impl TryFrom<storage::UsageSummary> for UsageSummaryResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::UsageSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            totals: value.totals.into(),
            errors: value.errors.into_iter().map(Into::into).collect(),
            recent_failures: value
                .recent_failures
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct LimitPolicyResponse {
    pub scope: String,
    pub subject_id: String,
    pub request_quota: Option<i64>,
    pub request_quota_mode: String,
    pub request_window_seconds: i64,
    pub token_quota: Option<i64>,
    pub token_quota_mode: String,
    pub token_window_seconds: i64,
    pub rate_limit_requests: Option<i64>,
    pub rate_limit_mode: String,
    pub rate_limit_window_seconds: i64,
    pub concurrency_limit: Option<i64>,
    pub concurrency_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<storage::LimitPolicy> for LimitPolicyResponse {
    fn from(value: storage::LimitPolicy) -> Self {
        Self {
            scope: value.scope,
            subject_id: value.subject_id,
            request_quota: value.request_quota,
            request_quota_mode: value.request_quota_mode,
            request_window_seconds: value.request_window_seconds,
            token_quota: value.token_quota,
            token_quota_mode: value.token_quota_mode,
            token_window_seconds: value.token_window_seconds,
            rate_limit_requests: value.rate_limit_requests,
            rate_limit_mode: value.rate_limit_mode,
            rate_limit_window_seconds: value.rate_limit_window_seconds,
            concurrency_limit: value.concurrency_limit,
            concurrency_mode: value.concurrency_mode,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct LimitBucketResponse {
    pub limit: Option<i64>,
    pub used: i64,
    pub remaining: Option<i64>,
    pub window_seconds: Option<i64>,
    pub reset_at: Option<String>,
}

impl From<storage::LimitBucketState> for LimitBucketResponse {
    fn from(value: storage::LimitBucketState) -> Self {
        Self {
            limit: value.limit,
            used: value.used,
            remaining: value.remaining,
            window_seconds: value.window_seconds,
            reset_at: value.reset_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ConcurrencyResponse {
    pub limit: Option<i64>,
    pub in_flight: i64,
    pub remaining: Option<i64>,
}

impl From<storage::ConcurrencyState> for ConcurrencyResponse {
    fn from(value: storage::ConcurrencyState) -> Self {
        Self {
            limit: value.limit,
            in_flight: value.in_flight,
            remaining: value.remaining,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct LimitSubjectResponse {
    pub scope: String,
    pub subject_id: String,
    pub policy: LimitPolicyResponse,
    pub effective_policy: LimitPolicyResponse,
    pub request_quota: LimitBucketResponse,
    pub token_budget: LimitBucketResponse,
    pub rate_limit: LimitBucketResponse,
    pub concurrency: ConcurrencyResponse,
}

impl From<storage::LimitSubjectState> for LimitSubjectResponse {
    fn from(value: storage::LimitSubjectState) -> Self {
        Self {
            scope: value.scope,
            subject_id: value.subject_id,
            policy: value.policy.into(),
            effective_policy: value.effective_policy.into(),
            request_quota: value.request_quota.into(),
            token_budget: value.token_budget.into(),
            rate_limit: value.rate_limit.into(),
            concurrency: value.concurrency.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UserLimitResponse {
    pub user: LimitSubjectResponse,
    pub current_key: Option<LimitSubjectResponse>,
    pub api_keys: Vec<LimitSubjectResponse>,
}

impl From<storage::UserLimitState> for UserLimitResponse {
    fn from(value: storage::UserLimitState) -> Self {
        Self {
            user: value.user.into(),
            current_key: value.current_key.map(Into::into),
            api_keys: value.api_keys.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AdminLimitResponse {
    pub system: LimitPolicyResponse,
    pub users: Vec<LimitSubjectResponse>,
    pub api_keys: Vec<LimitSubjectResponse>,
}

impl From<storage::AdminLimitState> for AdminLimitResponse {
    fn from(value: storage::AdminLimitState) -> Self {
        Self {
            system: value.system.into(),
            users: value.users.into_iter().map(Into::into).collect(),
            api_keys: value.api_keys.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ApiKeyUsageResponse {
    pub api_key: ApiKeyResponse,
    pub usage: UsageSummaryResponse,
    pub limits: Option<LimitSubjectResponse>,
}

impl TryFrom<storage::ApiKeyUsageSummary> for ApiKeyUsageResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::ApiKeyUsageSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            api_key: value.api_key.into(),
            usage: value.usage.try_into()?,
            limits: value.limits.map(Into::into),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsRequestBucketResponse {
    pub bucket: String,
    pub request_count: i64,
    pub error_count: i64,
}

impl From<storage::AnalyticsRequestBucket> for AnalyticsRequestBucketResponse {
    fn from(value: storage::AnalyticsRequestBucket) -> Self {
        Self {
            bucket: value.bucket,
            request_count: value.request_count,
            error_count: value.error_count,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsTokenBucketResponse {
    pub date: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

impl From<storage::AnalyticsTokenBucket> for AnalyticsTokenBucketResponse {
    fn from(value: storage::AnalyticsTokenBucket) -> Self {
        Self {
            date: value.date,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            request_count: value.request_count,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsDimensionResponse {
    pub id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
    pub share: f64,
}

impl From<storage::AnalyticsDimensionShare> for AnalyticsDimensionResponse {
    fn from(value: storage::AnalyticsDimensionShare) -> Self {
        Self {
            id: value.id,
            request_count: value.request_count,
            error_count: value.error_count,
            total_tokens: value.total_tokens,
            latency_ms_sum: value.latency_ms_sum,
            share: value.share,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsUpstreamErrorResponse {
    pub upstream_id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub error_rate: f64,
    pub avg_latency_ms: Option<f64>,
}

impl From<storage::AnalyticsUpstreamErrorRate> for AnalyticsUpstreamErrorResponse {
    fn from(value: storage::AnalyticsUpstreamErrorRate) -> Self {
        Self {
            upstream_id: value.upstream_id,
            request_count: value.request_count,
            error_count: value.error_count,
            error_rate: value.error_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsUserErrorResponse {
    pub user_id: String,
    pub request_count: i64,
    pub error_count: i64,
    pub error_rate: f64,
    pub avg_latency_ms: Option<f64>,
}

impl From<storage::AnalyticsUserErrorRate> for AnalyticsUserErrorResponse {
    fn from(value: storage::AnalyticsUserErrorRate) -> Self {
        Self {
            user_id: value.user_id,
            request_count: value.request_count,
            error_count: value.error_count,
            error_rate: value.error_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsLatencyTrendResponse {
    pub bucket: String,
    pub request_count: i64,
    pub error_count: i64,
    pub avg_latency_ms: Option<f64>,
}

impl From<storage::AnalyticsLatencyTrendBucket> for AnalyticsLatencyTrendResponse {
    fn from(value: storage::AnalyticsLatencyTrendBucket) -> Self {
        Self {
            bucket: value.bucket,
            request_count: value.request_count,
            error_count: value.error_count,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsLatencyBucketResponse {
    pub label: String,
    pub min_ms: i64,
    pub max_ms: Option<i64>,
    pub request_count: i64,
    pub error_count: i64,
}

impl From<storage::AnalyticsLatencyBucket> for AnalyticsLatencyBucketResponse {
    fn from(value: storage::AnalyticsLatencyBucket) -> Self {
        Self {
            label: value.label,
            min_ms: value.min_ms,
            max_ms: value.max_ms,
            request_count: value.request_count,
            error_count: value.error_count,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AnalyticsResponse {
    pub generated_at: String,
    pub requests_24h: Vec<AnalyticsRequestBucketResponse>,
    pub token_usage_7d: Vec<AnalyticsTokenBucketResponse>,
    pub model_share: Vec<AnalyticsDimensionResponse>,
    pub upstream_error_rate: Vec<AnalyticsUpstreamErrorResponse>,
    pub user_error_rate: Vec<AnalyticsUserErrorResponse>,
    pub latency_trend: Vec<AnalyticsLatencyTrendResponse>,
    pub latency_buckets: Vec<AnalyticsLatencyBucketResponse>,
}

impl From<storage::AnalyticsSnapshot> for AnalyticsResponse {
    fn from(value: storage::AnalyticsSnapshot) -> Self {
        Self {
            generated_at: value.generated_at,
            requests_24h: value.requests_24h.into_iter().map(Into::into).collect(),
            token_usage_7d: value.token_usage_7d.into_iter().map(Into::into).collect(),
            model_share: value.model_share.into_iter().map(Into::into).collect(),
            upstream_error_rate: value
                .upstream_error_rate
                .into_iter()
                .map(Into::into)
                .collect(),
            user_error_rate: value.user_error_rate.into_iter().map(Into::into).collect(),
            latency_trend: value.latency_trend.into_iter().map(Into::into).collect(),
            latency_buckets: value.latency_buckets.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct LatencyMetricsResponse {
    pub sum_ms: i64,
    pub avg_ms: Option<f64>,
}

impl From<storage::LatencyMetrics> for LatencyMetricsResponse {
    fn from(value: storage::LatencyMetrics) -> Self {
        Self {
            sum_ms: value.sum_ms,
            avg_ms: value.avg_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TokenUsageMetricsResponse {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

impl From<storage::TokenUsageMetrics> for TokenUsageMetricsResponse {
    fn from(value: storage::TokenUsageMetrics) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UpstreamHealthResponse {
    pub upstream_id: String,
    pub name: String,
    pub enabled: bool,
    pub last_health_status: String,
    pub last_health_checked_at: Option<String>,
    pub last_degraded_at: Option<String>,
    pub last_down_at: Option<String>,
    pub recent_error_samples: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency_ms_sum: i64,
    pub total_tokens: i64,
}

impl TryFrom<storage::UpstreamHealthMetrics> for UpstreamHealthResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::UpstreamHealthMetrics) -> Result<Self, Self::Error> {
        Ok(Self {
            upstream_id: value.upstream_id,
            name: value.name,
            enabled: response_bool(value.enabled)?,
            last_health_status: value.last_health_status,
            last_health_checked_at: value.last_health_checked_at,
            last_degraded_at: value.last_degraded_at,
            last_down_at: value.last_down_at,
            recent_error_samples: value.recent_error_samples,
            request_count: value.request_count,
            error_count: value.error_count,
            latency_ms_sum: value.latency_ms_sum,
            total_tokens: value.total_tokens,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct GatewayMetricsResponse {
    pub generated_at: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency: LatencyMetricsResponse,
    pub token_usage: TokenUsageMetricsResponse,
    pub upstream_health: Vec<UpstreamHealthResponse>,
}

impl TryFrom<storage::GatewayMetrics> for GatewayMetricsResponse {
    type Error = super::ApiError;

    fn try_from(value: storage::GatewayMetrics) -> Result<Self, Self::Error> {
        Ok(Self {
            generated_at: value.generated_at,
            request_count: value.request_count,
            error_count: value.error_count,
            latency: value.latency.into(),
            token_usage: value.token_usage.into(),
            upstream_health: value
                .upstream_health
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RetentionResponse {
    pub request_logs_deleted: u64,
    pub daily_usage_deleted: u64,
    pub request_log_cutoff: Option<String>,
    pub daily_usage_cutoff: Option<String>,
}

impl From<storage::RetentionResult> for RetentionResponse {
    fn from(value: storage::RetentionResult) -> Self {
        Self {
            request_logs_deleted: value.request_logs_deleted,
            daily_usage_deleted: value.daily_usage_deleted,
            request_log_cutoff: value.request_log_cutoff,
            daily_usage_cutoff: value.daily_usage_cutoff,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RuntimeConfigFieldResponse {
    pub key: &'static str,
    pub label: &'static str,
    pub value_type: &'static str,
    pub validation: serde_json::Value,
    pub environment_variable: &'static str,
    pub unit: Option<&'static str>,
    pub value: serde_json::Value,
    pub source: &'static str,
    pub database_value: Option<serde_json::Value>,
    pub environment_value: Option<serde_json::Value>,
    pub default_value: serde_json::Value,
    pub editable: bool,
    pub live_reload: bool,
    pub requires_restart: bool,
}

impl From<storage::RuntimeConfigField> for RuntimeConfigFieldResponse {
    fn from(value: storage::RuntimeConfigField) -> Self {
        Self {
            key: value.key,
            label: value.label,
            value_type: value.value_type,
            validation: value.validation,
            environment_variable: value.environment_variable,
            unit: value.unit,
            value: value.value,
            source: value.source,
            database_value: value.database_value,
            environment_value: value.environment_value,
            default_value: value.default_value,
            editable: value.editable,
            live_reload: value.live_reload,
            requires_restart: value.requires_restart,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    const FIXTURE: &str = include_str!("../../fixtures/stage5-http-contracts.json");

    #[test]
    fn response_dtos_match_stage_five_fixtures() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let actual = json!({
            "upstream": UpstreamResponse::try_from_record(sample_upstream(), 999_999).unwrap(),
            "model": ModelResponse::try_from(sample_model()).unwrap(),
            "model_mapping": ModelMappingResponse::try_from(sample_model_mapping()).unwrap(),
            "request_log": RequestLogResponse::try_from(sample_request_log()).unwrap(),
            "analytics": AnalyticsResponse::from(sample_analytics()),
            "gateway_metrics": GatewayMetricsResponse::try_from(sample_gateway_metrics()).unwrap(),
        });
        assert_eq!(actual, fixture["after"]);
    }

    #[test]
    fn only_approved_integer_to_boolean_differences_exist() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut actual = BTreeSet::new();
        collect_differences("", &fixture["before"], &fixture["after"], &mut actual);
        let approved = fixture["approved_differences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, approved);

        for path in actual {
            let before = value_at_path(&fixture["before"], &path);
            let after = value_at_path(&fixture["after"], &path);
            assert!(matches!(before.as_i64(), Some(0 | 1)), "{path}");
            assert_eq!(after.as_bool(), Some(before.as_i64() == Some(1)), "{path}");
        }
    }

    #[test]
    fn every_sqlite_boolean_response_field_rejects_corrupt_values() {
        for value in [0, 1, 2, -1] {
            let expected = match value {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            };

            let mut upstream = sample_upstream();
            upstream.enabled = value;
            assert_boolean_result(
                UpstreamResponse::try_from_record(upstream, 999_999).map(|dto| dto.enabled),
                expected,
                "upstream.enabled",
            );

            let mut upstream = sample_upstream();
            upstream.timeout_ms_is_explicit = value;
            let converted = UpstreamResponse::try_from_record(upstream, 999_999);
            match expected {
                Some(expected) => {
                    let converted = converted.unwrap();
                    assert_eq!(converted.timeout_ms_is_explicit, expected);
                    assert_eq!(
                        converted.timeout_ms,
                        if expected {
                            crate::config::default_request_timeout_ms()
                        } else {
                            999_999
                        }
                    );
                }
                None => assert!(converted.is_err(), "upstream.timeout_ms_is_explicit"),
            }

            let mut model = sample_model();
            model.enabled = value;
            assert_boolean_result(
                ModelResponse::try_from(model).map(|dto| dto.enabled),
                expected,
                "model.enabled",
            );

            let mut model = sample_model();
            model.visible_to_users = value;
            assert_boolean_result(
                ModelResponse::try_from(model).map(|dto| dto.visible_to_users),
                expected,
                "model.visible_to_users",
            );

            let mut mapping = sample_model_mapping();
            mapping.enabled = value;
            assert_boolean_result(
                ModelMappingResponse::try_from(mapping).map(|dto| dto.enabled),
                expected,
                "model_mapping.enabled",
            );

            let mut request_log = sample_request_log();
            request_log.stream = value;
            assert_boolean_result(
                RequestLogResponse::try_from(request_log).map(|dto| dto.stream),
                expected,
                "request_log.stream",
            );

            let mut metrics = sample_gateway_metrics();
            metrics.upstream_health[0].enabled = value;
            assert_boolean_result(
                GatewayMetricsResponse::try_from(metrics).map(|dto| dto.upstream_health[0].enabled),
                expected,
                "gateway_metrics.upstream_health[].enabled",
            );
        }
    }

    #[test]
    fn unchanged_response_dto_conversions_preserve_json() {
        let authenticated = auth::AuthenticatedUser {
            user_id: "user-1".into(),
            api_key_id: "key-1".into(),
            key_prefix: "prefix".into(),
            email: "user@example.com".into(),
            role: "user".into(),
        };
        assert_eq!(
            serde_json::to_value(AuthenticatedUserResponse::from(authenticated)).unwrap(),
            json!({
                "user_id": "user-1",
                "api_key_id": "key-1",
                "key_prefix": "prefix",
                "email": "user@example.com",
                "role": "user"
            })
        );

        let user = sample_user();
        assert_same_json(&user, UserResponse::from(user.clone()));
        let api_key = sample_api_key();
        assert_same_json(&api_key, ApiKeyResponse::from(api_key.clone()));
        let daily_usage = sample_daily_usage();
        assert_same_json(&daily_usage, DailyUsageResponse::from(daily_usage.clone()));

        let usage_summary = sample_usage_summary();
        assert_same_json(
            &usage_summary,
            UsageSummaryResponse::try_from(usage_summary.clone()).unwrap(),
        );
        let limit_subject = sample_limit_subject();
        assert_same_json(
            &limit_subject,
            LimitSubjectResponse::from(limit_subject.clone()),
        );
        let user_limits = storage::UserLimitState {
            user: limit_subject.clone(),
            current_key: Some(limit_subject.clone()),
            api_keys: vec![limit_subject.clone()],
        };
        assert_same_json(&user_limits, UserLimitResponse::from(user_limits.clone()));
        let admin_limits = storage::AdminLimitState {
            system: sample_limit_policy(),
            users: vec![limit_subject.clone()],
            api_keys: vec![limit_subject.clone()],
        };
        assert_same_json(
            &admin_limits,
            AdminLimitResponse::from(admin_limits.clone()),
        );
        let api_key_usage = storage::ApiKeyUsageSummary {
            api_key,
            usage: usage_summary,
            limits: Some(limit_subject),
        };
        assert_same_json(
            &api_key_usage,
            ApiKeyUsageResponse::try_from(api_key_usage.clone()).unwrap(),
        );

        let analytics = sample_analytics();
        assert_same_json(&analytics, AnalyticsResponse::from(analytics.clone()));
        let retention = storage::RetentionResult {
            request_logs_deleted: 2,
            daily_usage_deleted: 3,
            request_log_cutoff: Some("2026-01-01T00:00:00.000Z".into()),
            daily_usage_cutoff: None,
        };
        assert_same_json(&retention, RetentionResponse::from(retention.clone()));
        let runtime_field = storage::RuntimeConfigField {
            key: "default_request_timeout_ms",
            label: "Default request timeout",
            value_type: "integer",
            validation: json!({ "minimum": 1 }),
            environment_variable: "CODEX_GATEWAY_DEFAULT_REQUEST_TIMEOUT_MS",
            unit: Some("ms"),
            value: json!(crate::config::default_request_timeout_ms()),
            source: "default",
            database_value: None,
            environment_value: None,
            default_value: json!(crate::config::default_request_timeout_ms()),
            editable: true,
            live_reload: true,
            requires_restart: false,
        };
        assert_same_json(
            &runtime_field,
            RuntimeConfigFieldResponse::from(runtime_field.clone()),
        );
    }

    #[test]
    fn sensitive_persistence_records_keep_intentional_trait_hardening() {
        for (source, record) in [
            (include_str!("../storage/users.rs"), "UserCredentials"),
            (include_str!("../storage/upstreams.rs"), "Upstream"),
            (include_str!("../auth/persistence.rs"), "ApiKeyRecord"),
        ] {
            let declaration = format!("struct {record}");
            let declaration_at = source.find(&declaration).expect("sensitive record exists");
            let derive_at = source[..declaration_at]
                .rfind("#[derive(")
                .expect("sensitive record has an explicit derive list");
            let derives = &source[derive_at..declaration_at];
            for forbidden in ["Debug", "Deserialize", "Serialize"] {
                assert!(
                    !derives.contains(forbidden),
                    "{record} derives forbidden {forbidden}"
                );
                assert!(
                    !source.contains(&format!("impl {forbidden} for {record}")),
                    "{record} manually implements forbidden {forbidden}"
                );
            }
        }
    }

    fn collect_differences(
        path: &str,
        before: &Value,
        after: &Value,
        differences: &mut BTreeSet<String>,
    ) {
        match (before, after) {
            (Value::Object(before), Value::Object(after)) => {
                let keys = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
                for key in keys {
                    let child_path = if path.is_empty() {
                        key.to_string()
                    } else {
                        format!("{path}.{key}")
                    };
                    match (before.get(key), after.get(key)) {
                        (Some(before), Some(after)) => {
                            collect_differences(&child_path, before, after, differences);
                        }
                        _ => {
                            differences.insert(child_path);
                        }
                    }
                }
            }
            (Value::Array(before), Value::Array(after)) => {
                let length = before.len().max(after.len());
                for index in 0..length {
                    let child_path = format!("{path}[{index}]");
                    match (before.get(index), after.get(index)) {
                        (Some(before), Some(after)) => {
                            collect_differences(&child_path, before, after, differences);
                        }
                        _ => {
                            differences.insert(child_path);
                        }
                    }
                }
            }
            _ if before != after => {
                differences.insert(path.to_string());
            }
            _ => {}
        }
    }

    fn value_at_path<'a>(root: &'a Value, path: &str) -> &'a Value {
        let mut value = root;
        for segment in path.split('.') {
            if let Some((key, index)) = segment.split_once('[') {
                value = &value[key];
                let index = index.trim_end_matches(']').parse::<usize>().unwrap();
                value = &value[index];
            } else {
                value = &value[segment];
            }
        }
        value
    }

    fn assert_same_json<T: Serialize, U: Serialize>(before: &T, after: U) {
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    fn assert_boolean_result(
        actual: Result<bool, super::super::ApiError>,
        expected: Option<bool>,
        field: &str,
    ) {
        match expected {
            Some(expected) => assert_eq!(actual.unwrap(), expected, "{field}"),
            None => assert!(actual.is_err(), "{field}"),
        }
    }

    fn sample_user() -> storage::User {
        storage::User {
            id: "user-1".into(),
            email: "user@example.com".into(),
            role: "user".into(),
            status: "active".into(),
            display_name: None,
            created_at: "2026-07-12T00:00:00.000Z".into(),
            updated_at: "2026-07-12T00:00:00.000Z".into(),
            last_login_at: None,
        }
    }

    fn sample_api_key() -> storage::ApiKeySummary {
        storage::ApiKeySummary {
            id: "key-1".into(),
            user_id: "user-1".into(),
            name: "test".into(),
            key_prefix: "prefix".into(),
            status: "active".into(),
            last_used_at: None,
            expires_at: None,
            created_at: "2026-07-12T00:00:00.000Z".into(),
            revoked_at: None,
        }
    }

    fn sample_daily_usage() -> storage::DailyUsageRow {
        storage::DailyUsageRow {
            date: "2026-07-12".into(),
            user_id: "user-1".into(),
            api_key_id: "key-1".into(),
            model_id: Some("model-1".into()),
            upstream_id: Some("upstream-1".into()),
            request_count: 1,
            error_count: 0,
            stream_count: 1,
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            latency_ms_sum: 30,
        }
    }

    fn sample_usage_summary() -> storage::UsageSummary {
        storage::UsageSummary {
            totals: storage::UsageTotals {
                request_count: 1,
                error_count: 0,
                stream_count: 1,
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
                latency_ms_sum: 30,
                error_rate: 0.0,
            },
            errors: vec![storage::ErrorSummaryRow {
                error_code: "none".into(),
                status_code: None,
                count: 0,
                last_seen_at: None,
            }],
            recent_failures: Vec::new(),
        }
    }

    fn sample_limit_policy() -> storage::LimitPolicy {
        storage::LimitPolicy {
            scope: "system".into(),
            subject_id: "".into(),
            request_quota: Some(100),
            request_quota_mode: "override".into(),
            request_window_seconds: 3600,
            token_quota: None,
            token_quota_mode: "unlimited".into(),
            token_window_seconds: 3600,
            rate_limit_requests: Some(10),
            rate_limit_mode: "override".into(),
            rate_limit_window_seconds: 60,
            concurrency_limit: Some(2),
            concurrency_mode: "override".into(),
            created_at: "2026-07-12T00:00:00.000Z".into(),
            updated_at: "2026-07-12T00:00:00.000Z".into(),
        }
    }

    fn sample_limit_subject() -> storage::LimitSubjectState {
        let bucket = storage::LimitBucketState {
            limit: Some(100),
            used: 1,
            remaining: Some(99),
            window_seconds: Some(3600),
            reset_at: None,
        };
        storage::LimitSubjectState {
            scope: "user".into(),
            subject_id: "user-1".into(),
            policy: sample_limit_policy(),
            effective_policy: sample_limit_policy(),
            request_quota: bucket.clone(),
            token_budget: bucket.clone(),
            rate_limit: bucket,
            concurrency: storage::ConcurrencyState {
                limit: Some(2),
                in_flight: 1,
                remaining: Some(1),
            },
        }
    }

    fn sample_analytics() -> storage::AnalyticsSnapshot {
        storage::AnalyticsSnapshot {
            generated_at: "2026-07-12T00:00:00.000Z".into(),
            requests_24h: vec![storage::AnalyticsRequestBucket {
                bucket: "hour-1".into(),
                request_count: 11,
                error_count: 12,
            }],
            token_usage_7d: vec![storage::AnalyticsTokenBucket {
                date: "2026-07-11".into(),
                prompt_tokens: 21,
                completion_tokens: 22,
                total_tokens: 43,
                request_count: 24,
            }],
            model_share: vec![storage::AnalyticsDimensionShare {
                id: Some("model-analytics".into()),
                request_count: 31,
                error_count: 32,
                total_tokens: 33,
                latency_ms_sum: 34,
                share: 0.35,
            }],
            upstream_error_rate: vec![storage::AnalyticsUpstreamErrorRate {
                upstream_id: Some("upstream-analytics".into()),
                request_count: 41,
                error_count: 42,
                error_rate: 0.43,
                avg_latency_ms: Some(44.5),
            }],
            user_error_rate: vec![storage::AnalyticsUserErrorRate {
                user_id: "user-analytics".into(),
                request_count: 51,
                error_count: 52,
                error_rate: 0.53,
                avg_latency_ms: Some(54.5),
            }],
            latency_trend: vec![storage::AnalyticsLatencyTrendBucket {
                bucket: "trend-1".into(),
                request_count: 61,
                error_count: 62,
                avg_latency_ms: Some(63.5),
            }],
            latency_buckets: vec![storage::AnalyticsLatencyBucket {
                label: "64-128ms".into(),
                min_ms: 64,
                max_ms: Some(128),
                request_count: 65,
                error_count: 66,
            }],
        }
    }

    fn sample_upstream() -> storage::Upstream {
        storage::Upstream {
            id: "upstream-1".into(),
            name: "primary".into(),
            base_url: "https://example.invalid".into(),
            api_key_ciphertext: "ciphertext-placeholder".into(),
            api_key_secret_version: 1,
            enabled: 1,
            priority: 10,
            weight: 2,
            timeout_ms: crate::config::default_request_timeout_ms(),
            timeout_ms_is_explicit: 1,
            max_retries: 1,
            health_check_path: "/v1/models".into(),
            last_health_status: "unknown".into(),
            last_health_checked_at: None,
            health_status_changed_at: None,
            last_degraded_at: None,
            last_down_at: None,
            recent_error_samples: "[]".into(),
            created_at: "2026-07-12T00:00:00.000Z".into(),
            updated_at: "2026-07-12T00:00:00.000Z".into(),
        }
    }

    fn sample_model() -> storage::Model {
        storage::Model {
            id: "model-1".into(),
            public_name: "codex-mini".into(),
            description: None,
            enabled: 1,
            visible_to_users: 1,
            created_at: "2026-07-12T00:00:00.000Z".into(),
            updated_at: "2026-07-12T00:00:00.000Z".into(),
        }
    }

    fn sample_model_mapping() -> storage::UpstreamModel {
        storage::UpstreamModel {
            id: "mapping-1".into(),
            model_id: "model-1".into(),
            upstream_id: "upstream-1".into(),
            upstream_model_name: "upstream-codex-mini".into(),
            enabled: 1,
            priority: 10,
            weight: 2,
            created_at: "2026-07-12T00:00:00.000Z".into(),
            updated_at: "2026-07-12T00:00:00.000Z".into(),
        }
    }

    fn sample_request_log() -> storage::RequestLogRow {
        storage::RequestLogRow {
            id: "log-1".into(),
            request_id: "request-1".into(),
            user_id: "user-1".into(),
            api_key_id: "key-1".into(),
            model_id: Some("model-1".into()),
            upstream_id: Some("upstream-1".into()),
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(200),
            error_code: None,
            stream: 1,
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            usage_source: "upstream".into(),
            input_chars: 10,
            output_chars: 20,
            latency_ms: 30,
            started_at: "2026-07-12T00:00:00.000Z".into(),
            finished_at: Some("2026-07-12T00:00:00.030Z".into()),
            client_ip_hash: None,
            user_agent: None,
            upstream_response_id: Some("response-1".into()),
            upstream_status: Some("completed".into()),
            client_metadata_sanitized: None,
            route_strategy: Some("priority".into()),
            route_decision_json: Some("{}".into()),
        }
    }

    fn sample_gateway_metrics() -> storage::GatewayMetrics {
        storage::GatewayMetrics {
            generated_at: "2026-07-12T00:00:00.000Z".into(),
            request_count: 1,
            error_count: 0,
            latency: storage::LatencyMetrics {
                sum_ms: 71,
                avg_ms: Some(72.5),
            },
            token_usage: storage::TokenUsageMetrics {
                prompt_tokens: 73,
                completion_tokens: 74,
                total_tokens: 147,
            },
            upstream_health: vec![storage::UpstreamHealthMetrics {
                upstream_id: "upstream-1".into(),
                name: "primary".into(),
                enabled: 1,
                last_health_status: "healthy".into(),
                last_health_checked_at: Some("2026-07-12T00:00:00.000Z".into()),
                last_degraded_at: None,
                last_down_at: None,
                recent_error_samples: "[]".into(),
                request_count: 75,
                error_count: 76,
                latency_ms_sum: 77,
                total_tokens: 78,
            }],
        }
    }
}
