mod analytics;
mod api_keys;
mod audit;
mod db;
mod limits;
mod models;
mod request_logs;
mod runtime_config;
mod upstreams;
mod users;

pub(crate) use analytics::api_key_usage_summary_at;
pub use analytics::{
    AnalyticsDimensionShare, AnalyticsLatencyBucket, AnalyticsLatencyTrendBucket,
    AnalyticsRequestBucket, AnalyticsSnapshot, AnalyticsTokenBucket, AnalyticsUpstreamErrorRate,
    AnalyticsUserErrorRate, ApiKeyUsageSummary, DailyUsageFilters, DailyUsageRow, ErrorSummaryRow,
    GatewayMetrics, LatencyMetrics, TokenUsageMetrics, UpstreamHealthMetrics, UsageSummary,
    UsageTotals, analytics_snapshot, api_key_usage_summary, gateway_metrics, list_daily_usage,
    list_daily_usage_filtered, usage_summary,
};
pub use api_keys::{
    ApiKeySummary, CreateApiKey, create_api_key, create_api_key_conn, get_api_key, list_api_keys,
    list_api_keys_for_user, set_api_key_status, set_api_key_status_conn,
};
pub use audit::{
    AdminAuditInsert, AdminAuditLog, insert_admin_audit_log, list_admin_audit_logs,
    with_admin_audit,
};
pub(crate) use db::sqlite_bool;
pub use db::{connect_and_migrate, now_string};
pub use limits::{
    AdminLimitState, ConcurrencyState, LimitAdmission, LimitAdmissionError, LimitBucketState,
    LimitPatchValue, LimitPolicy, LimitPolicyPatch, LimitRejection, LimitSubjectState,
    UserLimitState, admin_limit_state, admit_limited_request, finalize_limit_admission,
    get_limit_policy, upsert_limit_policy, upsert_limit_policy_conn, user_limit_state,
};
pub(crate) use limits::{
    admin_limit_state_at, admit_limited_request_with_clock, finalize_limit_admission_with_clock,
    user_limit_state_at,
};
pub use models::{
    Model, UpdateModel, UpdateModelMapping, UpsertModel, UpsertModelMapping, UpstreamModel,
    create_model, create_model_conn, create_upstream_model, create_upstream_model_conn, get_model,
    get_upstream_model, list_models, list_upstream_models_for_model, list_visible_models,
    update_model, update_model_conn, update_upstream_model, update_upstream_model_conn,
};
pub use request_logs::{
    RequestLogFilters, RequestLogInsert, RequestLogRow, RetentionPolicy, RetentionResult,
    apply_retention, apply_retention_at, apply_retention_conn, insert_request_log,
    list_request_logs, list_request_logs_filtered,
};
pub use runtime_config::{
    ConfigPatchValue, ResolvedRuntimeConfig, RuntimeConfigField, SystemConfig, SystemConfigPatch,
    get_system_config, resolve_runtime_config, runtime_config, upsert_system_config,
    upsert_system_config_conn,
};
pub use upstreams::{
    TimeoutPatchValue, UpdateUpstream, UpsertUpstream, Upstream, UpstreamRecord, create_upstream,
    create_upstream_conn, get_upstream, list_enabled_upstreams, list_upstreams,
    record_upstream_health, record_upstream_health_conn, update_upstream, update_upstream_conn,
    update_upstream_health, upgrade_legacy_upstream_secrets,
};
pub(crate) use users::reset_user_password_command_conn;
pub use users::{
    CreateUser, ResetPassword, UpdateUser, User, UserCredentials, UserCredentialsRecord,
    ensure_bootstrap_admin, ensure_user, ensure_user_conn, find_user_credentials_by_email,
    get_user, list_users, mark_user_login, reset_user_password, reset_user_password_conn,
    seed_bootstrap_admin, update_user, update_user_conn,
};

#[cfg(test)]
mod tests;
