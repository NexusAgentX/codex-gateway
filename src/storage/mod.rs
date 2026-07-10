use std::{path::Path, str::FromStr};

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sqlx::{
    FromRow, QueryBuilder, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{auth, config::Config, usage::UsageSnapshot};

pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<SqlitePool> {
    create_sqlite_parent(database_url)?;
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("parsing database URL {database_url}"))?
        .create_if_missing(true)
        .foreign_keys(true);
    let max_connections = if database_url.contains(":memory:") {
        1
    } else {
        5
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .context("connecting SQLite database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running database migrations")?;
    clear_stale_limit_inflight(&pool).await?;
    Ok(pool)
}

fn create_sqlite_parent(database_url: &str) -> anyhow::Result<()> {
    let Some(path) = database_url.strip_prefix("sqlite://") else {
        return Ok(());
    };
    if path == ":memory:" || path.starts_with("file:") {
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating SQLite database directory {}", parent.display()))?;
    }
    Ok(())
}

pub async fn seed_bootstrap_admin(pool: &SqlitePool, config: &Config) -> anyhow::Result<()> {
    let Some(email) = &config.admin_email else {
        return Ok(());
    };
    let user_id = ensure_bootstrap_admin(
        pool,
        email,
        config.admin_password.as_deref(),
        Some("Bootstrap Admin"),
    )
    .await?;

    if let Some(key) = &config.bootstrap_admin_key {
        let prepared = auth::prepare_existing_api_key(&config.app_secret, key)?;
        create_or_replace_named_key(
            pool,
            &user_id,
            "bootstrap-admin",
            &prepared.prefix,
            &prepared.hash,
        )
        .await?;
    }

    Ok(())
}

pub async fn upgrade_legacy_upstream_secrets(
    pool: &SqlitePool,
    config: &Config,
) -> anyhow::Result<u64> {
    let legacy_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, api_key_ciphertext
         FROM upstreams
         WHERE api_key_secret_version = 0",
    )
    .fetch_all(pool)
    .await?;

    let mut upgraded = 0;
    for (id, stored_key) in legacy_rows {
        if crate::secrets::is_encrypted_secret(&stored_key) {
            continue;
        }
        let encrypted_key = crate::secrets::encrypt_upstream_api_key(
            &config.app_secret,
            config.secret_key_version,
            &stored_key,
        )?;
        let result = sqlx::query(
            "UPDATE upstreams
             SET api_key_ciphertext = ?, api_key_secret_version = ?, updated_at = ?
             WHERE id = ? AND api_key_secret_version = 0",
        )
        .bind(encrypted_key)
        .bind(config.secret_key_version)
        .bind(now_string())
        .bind(id)
        .execute(pool)
        .await?;
        upgraded += result.rows_affected();
    }
    Ok(upgraded)
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub display_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct UserCredentials {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub password: String,
    pub role: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateUser {
    pub role: Option<String>,
    pub status: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResetPassword {
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct ApiKeyRecord {
    pub api_key_id: String,
    pub user_id: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub key_status: String,
    pub expires_at: Option<String>,
    pub email: String,
    pub role: String,
    pub user_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct ApiKeySummary {
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

#[derive(Clone, Debug, Deserialize)]
pub struct CreateApiKey {
    pub name: String,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct Upstream {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub api_key_ciphertext: String,
    #[serde(skip_serializing)]
    pub api_key_secret_version: i64,
    pub enabled: i64,
    pub priority: i64,
    pub weight: i64,
    pub timeout_ms: i64,
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

#[derive(Clone, Debug, Deserialize)]
pub struct UpsertUpstream {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub timeout_ms: Option<i64>,
    pub max_retries: Option<i64>,
    pub health_check_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateUpstream {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub timeout_ms: Option<i64>,
    pub max_retries: Option<i64>,
    pub health_check_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct Model {
    pub id: String,
    pub public_name: String,
    pub description: Option<String>,
    pub enabled: i64,
    pub visible_to_users: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpsertModel {
    pub public_name: String,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub visible_to_users: Option<bool>,
    pub upstream_mappings: Option<Vec<UpsertModelMapping>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateModel {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub visible_to_users: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpsertModelMapping {
    pub upstream_id: String,
    pub upstream_model_name: String,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct UpstreamModel {
    pub id: String,
    pub model_id: String,
    pub upstream_id: String,
    pub upstream_model_name: String,
    pub enabled: i64,
    pub priority: i64,
    pub weight: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateModelMapping {
    pub upstream_id: Option<String>,
    pub upstream_model_name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct RequestLogRow {
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
    pub stream: i64,
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

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct DailyUsageRow {
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

#[derive(Clone, Debug, Default)]
pub struct DailyUsageFilters {
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageTotals {
    pub request_count: i64,
    pub error_count: i64,
    pub stream_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
    pub error_rate: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct ErrorSummaryRow {
    pub error_code: String,
    pub status_code: Option<i64>,
    pub count: i64,
    pub last_seen_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageSummary {
    pub totals: UsageTotals,
    pub errors: Vec<ErrorSummaryRow>,
    pub recent_failures: Vec<RequestLogRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiKeyUsageSummary {
    pub api_key: ApiKeySummary,
    pub usage: UsageSummary,
    pub limits: Option<LimitSubjectState>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestLogFilters {
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub status_code: Option<i64>,
    pub started_at_from: Option<String>,
    pub started_at_to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GatewayMetrics {
    pub generated_at: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency: LatencyMetrics,
    pub token_usage: TokenUsageMetrics,
    pub upstream_health: Vec<UpstreamHealthMetrics>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LatencyMetrics {
    pub sum_ms: i64,
    pub avg_ms: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenUsageMetrics {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct UpstreamHealthMetrics {
    pub upstream_id: String,
    pub name: String,
    pub enabled: i64,
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

#[derive(Clone, Debug)]
pub struct RetentionPolicy {
    pub request_log_retention_days: i64,
    pub daily_usage_retention_days: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetentionResult {
    pub request_logs_deleted: u64,
    pub daily_usage_deleted: u64,
    pub request_log_cutoff: Option<String>,
    pub daily_usage_cutoff: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RequestLogInsert {
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
    pub usage: UsageSnapshot,
    pub input_chars: i64,
    pub output_chars: i64,
    pub latency_ms: i64,
    pub started_at: String,
    pub finished_at: String,
    pub client_ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub client_metadata_sanitized: Option<String>,
    pub route_strategy: Option<String>,
    pub route_decision_json: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct AdminAuditLog {
    pub id: String,
    pub actor_user_id: String,
    pub actor_email: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub status: String,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AdminAuditInsert {
    pub actor_user_id: String,
    pub actor_email: String,
    pub action: &'static str,
    pub resource_type: &'static str,
    pub resource_id: Option<String>,
    pub status: &'static str,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct LimitPolicy {
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LimitPolicyPatch {
    #[serde(default)]
    pub request_quota: LimitPatchValue,
    pub request_window_seconds: Option<i64>,
    #[serde(default)]
    pub token_quota: LimitPatchValue,
    pub token_window_seconds: Option<i64>,
    #[serde(default)]
    pub rate_limit_requests: LimitPatchValue,
    pub rate_limit_window_seconds: Option<i64>,
    #[serde(default)]
    pub concurrency_limit: LimitPatchValue,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LimitPatchValue {
    #[default]
    Missing,
    Inherit,
    Clear,
    Set(i64),
}

impl<'de> Deserialize<'de> for LimitPatchValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(Self::Clear),
            serde_json::Value::Number(number) => number
                .as_i64()
                .map(Self::Set)
                .ok_or_else(|| serde::de::Error::custom("limit value must be an integer")),
            serde_json::Value::Object(object) => {
                let mode = object
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom("limit mode is required"))?;
                match mode {
                    "inherit" => Ok(Self::Inherit),
                    "unlimited" => Ok(Self::Clear),
                    "limited" => object
                        .get("value")
                        .and_then(serde_json::Value::as_i64)
                        .map(Self::Set)
                        .ok_or_else(|| {
                            serde::de::Error::custom("limited mode requires integer value")
                        }),
                    _ => Err(serde::de::Error::custom(
                        "limit mode must be inherit, limited, or unlimited",
                    )),
                }
            }
            _ => Err(serde::de::Error::custom(
                "limit value must be an integer, null, or mode object",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitBucketState {
    pub limit: Option<i64>,
    pub used: i64,
    pub remaining: Option<i64>,
    pub window_seconds: Option<i64>,
    pub reset_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConcurrencyState {
    pub limit: Option<i64>,
    pub in_flight: i64,
    pub remaining: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitSubjectState {
    pub scope: String,
    pub subject_id: String,
    pub policy: LimitPolicy,
    pub effective_policy: LimitPolicy,
    pub request_quota: LimitBucketState,
    pub token_budget: LimitBucketState,
    pub rate_limit: LimitBucketState,
    pub concurrency: ConcurrencyState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserLimitState {
    pub user: LimitSubjectState,
    pub current_key: Option<LimitSubjectState>,
    pub api_keys: Vec<LimitSubjectState>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdminLimitState {
    pub system: LimitPolicy,
    pub users: Vec<LimitSubjectState>,
    pub api_keys: Vec<LimitSubjectState>,
}

#[derive(Clone, Debug)]
pub struct LimitAdmission {
    pub usage_event_id: String,
    pub inflight_request_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitRejection {
    pub code: &'static str,
    pub message: String,
    pub scope: String,
    pub subject_id: String,
    pub limit_name: &'static str,
    pub limit: i64,
    pub used: i64,
    pub reset_at: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LimitAdmissionError {
    #[error("limit rejected")]
    Rejected(LimitRejection),
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

pub async fn ensure_bootstrap_admin(
    pool: &SqlitePool,
    email: &str,
    password: Option<&str>,
    display_name: Option<&str>,
) -> anyhow::Result<String> {
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    let now = now_string();
    if let Some((id,)) = existing {
        if let Some(password) = password {
            let password_hash = auth::hash_password(password)?;
            sqlx::query(
                "UPDATE users
                 SET password_hash = ?, role = 'admin', status = 'active',
                     display_name = COALESCE(?, display_name), updated_at = ?
                 WHERE id = ?",
            )
            .bind(password_hash)
            .bind(display_name)
            .bind(&now)
            .bind(&id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE users
                 SET role = 'admin', status = 'active',
                     display_name = COALESCE(?, display_name), updated_at = ?
                 WHERE id = ?",
            )
            .bind(display_name)
            .bind(&now)
            .bind(&id)
            .execute(pool)
            .await?;
        }
        return Ok(id);
    }

    let password_hash = auth::hash_password(password.unwrap_or("change-me-on-first-login"))?;
    let id = auth::new_id();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, status, display_name, created_at, updated_at)
         VALUES (?, ?, ?, 'admin', 'active', ?, ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn ensure_user(pool: &SqlitePool, input: &CreateUser) -> anyhow::Result<String> {
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(&input.email)
        .fetch_optional(pool)
        .await?;
    if let Some((id,)) = existing {
        return Ok(id);
    }

    let id = auth::new_id();
    let password_hash = auth::hash_password(&input.password)?;
    let now = now_string();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, status, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'active', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.email)
    .bind(password_hash)
    .bind(&input.role)
    .bind(&input.display_name)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn get_user(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as("SELECT id, email, role, status, display_name, created_at, updated_at, last_login_at FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_user(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateUser,
) -> sqlx::Result<Option<User>> {
    let Some(existing) = get_user(pool, id).await? else {
        return Ok(None);
    };
    let role = input.role.as_deref().unwrap_or(&existing.role);
    let status = input.status.as_deref().unwrap_or(&existing.status);
    let display_name = input
        .display_name
        .as_ref()
        .or(existing.display_name.as_ref());
    let now = now_string();
    sqlx::query(
        "UPDATE users
         SET role = ?, status = ?, display_name = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(role)
    .bind(status)
    .bind(display_name)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    get_user(pool, id).await
}

pub async fn reset_user_password(
    pool: &SqlitePool,
    id: &str,
    password: &str,
) -> anyhow::Result<bool> {
    let password_hash = auth::hash_password(password)?;
    let result = sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(password_hash)
        .bind(now_string())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_users(pool: &SqlitePool) -> sqlx::Result<Vec<User>> {
    sqlx::query_as("SELECT id, email, role, status, display_name, created_at, updated_at, last_login_at FROM users ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

pub async fn find_user_credentials_by_email(
    pool: &SqlitePool,
    email: &str,
) -> sqlx::Result<Option<UserCredentials>> {
    sqlx::query_as("SELECT id, email, password_hash, role, status FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn mark_user_login(pool: &SqlitePool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_login_at = ?, updated_at = ? WHERE id = ?")
        .bind(now_string())
        .bind(now_string())
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_api_key_by_prefix(
    pool: &SqlitePool,
    prefix: &str,
) -> sqlx::Result<Option<ApiKeyRecord>> {
    sqlx::query_as(
        "SELECT
            api_keys.id AS api_key_id,
            api_keys.user_id AS user_id,
            api_keys.key_prefix AS key_prefix,
            api_keys.key_hash AS key_hash,
            api_keys.status AS key_status,
            api_keys.expires_at AS expires_at,
            users.email AS email,
            users.role AS role,
            users.status AS user_status
         FROM api_keys
         JOIN users ON users.id = api_keys.user_id
         WHERE api_keys.key_prefix = ?",
    )
    .bind(prefix)
    .fetch_optional(pool)
    .await
}

pub async fn mark_api_key_used(pool: &SqlitePool, api_key_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
        .bind(now_string())
        .bind(api_key_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_api_key(
    pool: &SqlitePool,
    app_secret: &str,
    user_id: &str,
    input: &CreateApiKey,
) -> anyhow::Result<(ApiKeySummary, String)> {
    let prepared = auth::generate_api_key(app_secret);
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, status, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&input.name)
    .bind(&prepared.prefix)
    .bind(&prepared.hash)
    .bind(&input.expires_at)
    .bind(&now)
    .execute(pool)
    .await?;

    let summary = get_api_key(pool, &id)
        .await?
        .context("created API key not found")?;
    Ok((summary, prepared.plaintext))
}

async fn create_or_replace_named_key(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
    prefix: &str,
    hash: &str,
) -> anyhow::Result<()> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM api_keys WHERE user_id = ? AND name = ? LIMIT 1")
            .bind(user_id)
            .bind(name)
            .fetch_optional(pool)
            .await?;
    let now = now_string();
    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE api_keys
             SET key_prefix = ?, key_hash = ?, status = 'active',
                 expires_at = NULL, revoked_at = NULL
             WHERE id = ?",
        )
        .bind(prefix)
        .bind(hash)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        let id = auth::new_id();
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, status, created_at)
             VALUES (?, ?, ?, ?, ?, 'active', ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(prefix)
        .bind(hash)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn get_api_key(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_api_keys_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> sqlx::Result<Vec<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys WHERE user_id = ? ORDER BY created_at DESC")
        .bind(user_id)
        .fetch_all(pool)
        .await
}

pub async fn list_api_keys(pool: &SqlitePool) -> sqlx::Result<Vec<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

pub async fn set_api_key_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
) -> sqlx::Result<Option<ApiKeySummary>> {
    let revoked_at = if status == "revoked" {
        Some(now_string())
    } else {
        None
    };
    sqlx::query(
        "UPDATE api_keys
         SET status = ?, revoked_at = COALESCE(?, revoked_at)
         WHERE id = ?",
    )
    .bind(status)
    .bind(revoked_at)
    .bind(id)
    .execute(pool)
    .await?;
    get_api_key(pool, id).await
}

pub async fn list_upstreams(pool: &SqlitePool) -> sqlx::Result<Vec<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams ORDER BY priority, name")
        .fetch_all(pool)
        .await
}

pub async fn list_enabled_upstreams(pool: &SqlitePool) -> sqlx::Result<Vec<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams WHERE enabled = 1 ORDER BY priority, name")
        .fetch_all(pool)
        .await
}

pub async fn create_upstream(
    pool: &SqlitePool,
    app_secret: &str,
    secret_key_version: i64,
    input: &UpsertUpstream,
) -> anyhow::Result<Upstream> {
    let id = auth::new_id();
    let now = now_string();
    let encrypted_key =
        crate::secrets::encrypt_upstream_api_key(app_secret, secret_key_version, &input.api_key)?;
    sqlx::query(
        "INSERT INTO upstreams
         (id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, max_retries, health_check_path, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(input.base_url.trim_end_matches('/'))
    .bind(encrypted_key)
    .bind(secret_key_version)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(input.priority.unwrap_or(100))
    .bind(input.weight.unwrap_or(1).max(1))
    .bind(input.timeout_ms.unwrap_or(120_000))
    .bind(input.max_retries.unwrap_or(1))
    .bind(input.health_check_path.as_deref().unwrap_or("/v1/models"))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    get_upstream(pool, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created upstream not found"))
}

pub async fn get_upstream(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_upstream(
    pool: &SqlitePool,
    app_secret: &str,
    secret_key_version: i64,
    id: &str,
    input: &UpdateUpstream,
) -> anyhow::Result<Option<Upstream>> {
    let Some(existing) = get_upstream(pool, id).await? else {
        return Ok(None);
    };
    let name = input.name.as_deref().unwrap_or(&existing.name);
    let base_url = input
        .base_url
        .as_deref()
        .unwrap_or(&existing.base_url)
        .trim_end_matches('/');
    let (api_key, api_key_secret_version) = if let Some(api_key) = input.api_key.as_deref() {
        (
            crate::secrets::encrypt_upstream_api_key(app_secret, secret_key_version, api_key)?,
            secret_key_version,
        )
    } else {
        (
            existing.api_key_ciphertext.clone(),
            existing.api_key_secret_version,
        )
    };
    let enabled = input.enabled.map(bool_to_i64).unwrap_or(existing.enabled);
    let priority = input.priority.unwrap_or(existing.priority);
    let weight = input.weight.unwrap_or(existing.weight).max(1);
    let timeout_ms = input.timeout_ms.unwrap_or(existing.timeout_ms);
    let max_retries = input.max_retries.unwrap_or(existing.max_retries);
    let health_check_path = input
        .health_check_path
        .as_deref()
        .unwrap_or(&existing.health_check_path);
    let now = now_string();
    sqlx::query(
        "UPDATE upstreams
         SET name = ?, base_url = ?, api_key_ciphertext = ?, api_key_secret_version = ?, enabled = ?,
             priority = ?, weight = ?, timeout_ms = ?, max_retries = ?,
             health_check_path = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(name)
    .bind(base_url)
    .bind(api_key)
    .bind(api_key_secret_version)
    .bind(enabled)
    .bind(priority)
    .bind(weight)
    .bind(timeout_ms)
    .bind(max_retries)
    .bind(health_check_path)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(get_upstream(pool, id).await?)
}

pub async fn update_upstream_health(pool: &SqlitePool, id: &str, status: &str) -> sqlx::Result<()> {
    record_upstream_health(pool, id, status, None).await
}

pub async fn record_upstream_health(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error_sample: Option<&str>,
) -> sqlx::Result<()> {
    let existing: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT last_health_status, recent_error_samples, health_status_changed_at, last_degraded_at, last_down_at
         FROM upstreams
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some((
        previous_status,
        recent_error_samples,
        previous_changed_at,
        previous_degraded_at,
        previous_down_at,
    )) = existing
    else {
        return Ok(());
    };

    let now = now_string();
    let status_changed_at = (previous_status != status).then_some(now.clone());
    let degraded_at =
        (status == "degraded" && previous_status != "degraded").then_some(now.clone());
    let down_at = (status == "down" && previous_status != "down").then_some(now.clone());
    let recent_error_samples =
        append_recent_error_sample(&recent_error_samples, error_sample, status, &now);

    sqlx::query(
        "UPDATE upstreams
         SET last_health_status = ?,
             last_health_checked_at = ?,
             health_status_changed_at = ?,
             last_degraded_at = ?,
             last_down_at = ?,
             recent_error_samples = ?,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(&now)
    .bind(status_changed_at.or(previous_changed_at))
    .bind(degraded_at.or(previous_degraded_at))
    .bind(down_at.or(previous_down_at))
    .bind(recent_error_samples)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

fn append_recent_error_sample(
    existing: &str,
    error_sample: Option<&str>,
    status: &str,
    now: &str,
) -> String {
    let Some(error_sample) = error_sample.filter(|sample| !sample.is_empty()) else {
        return existing.to_string();
    };
    let mut samples = serde_json::from_str::<Vec<serde_json::Value>>(existing).unwrap_or_default();
    samples.push(serde_json::json!({
        "at": now,
        "status": status,
        "error": error_sample
    }));
    let keep_from = samples.len().saturating_sub(5);
    serde_json::Value::Array(samples.split_off(keep_from)).to_string()
}

pub async fn list_models(pool: &SqlitePool) -> sqlx::Result<Vec<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models ORDER BY public_name")
        .fetch_all(pool)
        .await
}

pub async fn list_visible_models(pool: &SqlitePool) -> sqlx::Result<Vec<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models WHERE enabled = 1 AND visible_to_users = 1 ORDER BY public_name")
        .fetch_all(pool)
        .await
}

pub async fn create_model(pool: &SqlitePool, input: &UpsertModel) -> sqlx::Result<Model> {
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO models (id, public_name, description, enabled, visible_to_users, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.public_name)
    .bind(&input.description)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(bool_to_i64(input.visible_to_users.unwrap_or(true)))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    if let Some(mappings) = &input.upstream_mappings {
        for mapping in mappings {
            create_upstream_model(pool, &id, mapping).await?;
        }
    }

    get_model(pool, &id).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_model(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_model(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateModel,
) -> sqlx::Result<Option<Model>> {
    let Some(existing) = get_model(pool, id).await? else {
        return Ok(None);
    };
    let description = input.description.as_ref().or(existing.description.as_ref());
    let enabled = input.enabled.map(bool_to_i64).unwrap_or(existing.enabled);
    let visible_to_users = input
        .visible_to_users
        .map(bool_to_i64)
        .unwrap_or(existing.visible_to_users);
    let now = now_string();
    sqlx::query(
        "UPDATE models
         SET description = ?, enabled = ?, visible_to_users = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(description)
    .bind(enabled)
    .bind(visible_to_users)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    get_model(pool, id).await
}

pub async fn create_upstream_model(
    pool: &SqlitePool,
    model_id: &str,
    input: &UpsertModelMapping,
) -> sqlx::Result<UpstreamModel> {
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO upstream_models
         (id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(model_id)
    .bind(&input.upstream_id)
    .bind(&input.upstream_model_name)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(input.priority.unwrap_or(100))
    .bind(input.weight.unwrap_or(1).max(1))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn get_upstream_model(
    pool: &SqlitePool,
    id: &str,
) -> sqlx::Result<Option<UpstreamModel>> {
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_upstream_models_for_model(
    pool: &SqlitePool,
    model_id: &str,
) -> sqlx::Result<Vec<UpstreamModel>> {
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE model_id = ? ORDER BY priority, id")
        .bind(model_id)
        .fetch_all(pool)
        .await
}

pub async fn update_upstream_model(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateModelMapping,
) -> sqlx::Result<Option<UpstreamModel>> {
    let Some(existing) = get_upstream_model(pool, id).await? else {
        return Ok(None);
    };
    let upstream_id = input
        .upstream_id
        .as_deref()
        .unwrap_or(&existing.upstream_id);
    let upstream_model_name = input
        .upstream_model_name
        .as_deref()
        .unwrap_or(&existing.upstream_model_name);
    let enabled = input.enabled.map(bool_to_i64).unwrap_or(existing.enabled);
    let priority = input.priority.unwrap_or(existing.priority);
    let weight = input.weight.unwrap_or(existing.weight).max(1);
    let now = now_string();
    sqlx::query(
        "UPDATE upstream_models
         SET upstream_id = ?, upstream_model_name = ?, enabled = ?,
             priority = ?, weight = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(upstream_id)
    .bind(upstream_model_name)
    .bind(enabled)
    .bind(priority)
    .bind(weight)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    get_upstream_model(pool, id).await
}

pub async fn list_request_logs(
    pool: &SqlitePool,
    user_id: Option<&str>,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let mut filters = RequestLogFilters::default();
    filters.user_id = user_id.map(str::to_string);
    filters.limit = Some(if user_id.is_some() { 200 } else { 500 });
    list_request_logs_filtered(pool, &filters).await
}

pub async fn list_request_logs_filtered(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new("SELECT * FROM request_logs");
    let mut has_where = false;
    if let Some(user_id) = &filters.user_id {
        push_where(&mut query, &mut has_where);
        query.push("user_id = ").push_bind(user_id);
    }
    if let Some(api_key_id) = &filters.api_key_id {
        push_where(&mut query, &mut has_where);
        query.push("api_key_id = ").push_bind(api_key_id);
    }
    if let Some(model_id) = &filters.model_id {
        push_where(&mut query, &mut has_where);
        query.push("model_id = ").push_bind(model_id);
    }
    if let Some(upstream_id) = &filters.upstream_id {
        push_where(&mut query, &mut has_where);
        query.push("upstream_id = ").push_bind(upstream_id);
    }
    if let Some(status_code) = filters.status_code {
        push_where(&mut query, &mut has_where);
        query.push("status_code = ").push_bind(status_code);
    }
    if let Some(started_at_from) = &filters.started_at_from {
        push_where(&mut query, &mut has_where);
        query.push("started_at >= ").push_bind(started_at_from);
    }
    if let Some(started_at_to) = &filters.started_at_to {
        push_where(&mut query, &mut has_where);
        query.push("started_at <= ").push_bind(started_at_to);
    }
    query.push(" ORDER BY started_at DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(500).clamp(1, 1000));
    query.build_query_as().fetch_all(pool).await
}

fn push_where(query: &mut QueryBuilder<'_, Sqlite>, has_where: &mut bool) {
    if *has_where {
        query.push(" AND ");
    } else {
        query.push(" WHERE ");
        *has_where = true;
    }
}

pub async fn list_daily_usage(
    pool: &SqlitePool,
    user_id: Option<&str>,
) -> sqlx::Result<Vec<DailyUsageRow>> {
    let mut filters = DailyUsageFilters::default();
    filters.user_id = user_id.map(str::to_string);
    filters.limit = Some(if user_id.is_some() { 90 } else { 500 });
    list_daily_usage_filtered(pool, &filters).await
}

pub async fn list_daily_usage_filtered(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<Vec<DailyUsageRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count,
                prompt_tokens, completion_tokens, total_tokens, latency_ms_sum
         FROM daily_usage",
    );
    push_daily_usage_filters(&mut query, filters);
    query.push(" ORDER BY date DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(500).clamp(1, 1000));
    query.build_query_as().fetch_all(pool).await
}

pub async fn usage_summary(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<UsageSummary> {
    let mut totals_query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT
            COALESCE(SUM(request_count), 0),
            COALESCE(SUM(error_count), 0),
            COALESCE(SUM(stream_count), 0),
            COALESCE(SUM(prompt_tokens), 0),
            COALESCE(SUM(completion_tokens), 0),
            COALESCE(SUM(total_tokens), 0),
            COALESCE(SUM(latency_ms_sum), 0)
         FROM daily_usage",
    );
    push_daily_usage_filters(&mut totals_query, filters);
    let totals: (i64, i64, i64, i64, i64, i64, i64) =
        totals_query.build_query_as().fetch_one(pool).await?;
    let error_rate = if totals.0 > 0 {
        totals.1 as f64 / totals.0 as f64
    } else {
        0.0
    };
    let request_filters = request_filters_from_usage(filters, Some(12));
    Ok(UsageSummary {
        totals: UsageTotals {
            request_count: totals.0,
            error_count: totals.1,
            stream_count: totals.2,
            prompt_tokens: totals.3,
            completion_tokens: totals.4,
            total_tokens: totals.5,
            latency_ms_sum: totals.6,
            error_rate,
        },
        errors: error_summary(pool, filters).await?,
        recent_failures: list_recent_failures(pool, &request_filters).await?,
    })
}

pub async fn api_key_usage_summary(
    pool: &SqlitePool,
    api_key: ApiKeySummary,
    include_limits: bool,
) -> sqlx::Result<ApiKeyUsageSummary> {
    let filters = DailyUsageFilters {
        user_id: Some(api_key.user_id.clone()),
        api_key_id: Some(api_key.id.clone()),
        limit: Some(90),
        ..DailyUsageFilters::default()
    };
    let limits = if include_limits {
        user_limit_state(pool, &api_key.user_id, Some(&api_key.id))
            .await?
            .current_key
    } else {
        None
    };
    Ok(ApiKeyUsageSummary {
        api_key,
        usage: usage_summary(pool, &filters).await?,
        limits,
    })
}

fn push_daily_usage_filters<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    filters: &'a DailyUsageFilters,
) {
    let mut has_where = false;
    if let Some(user_id) = &filters.user_id {
        push_where(query, &mut has_where);
        query.push("user_id = ").push_bind(user_id);
    }
    if let Some(api_key_id) = &filters.api_key_id {
        push_where(query, &mut has_where);
        query.push("api_key_id = ").push_bind(api_key_id);
    }
    if let Some(model_id) = &filters.model_id {
        push_where(query, &mut has_where);
        query.push("model_id = ").push_bind(model_id);
    }
    if let Some(upstream_id) = &filters.upstream_id {
        push_where(query, &mut has_where);
        query.push("upstream_id = ").push_bind(upstream_id);
    }
    if let Some(date_from) = &filters.date_from {
        push_where(query, &mut has_where);
        query.push("date >= ").push_bind(date_from);
    }
    if let Some(date_to) = &filters.date_to {
        push_where(query, &mut has_where);
        query.push("date <= ").push_bind(date_to);
    }
}

async fn list_recent_failures(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new("SELECT * FROM request_logs");
    push_request_log_filter_where(&mut query, filters);
    query.push(if request_log_filters_empty(filters) {
        " WHERE "
    } else {
        " AND "
    });
    query.push("COALESCE(status_code, 500) >= 400 ORDER BY started_at DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(12).clamp(1, 100));
    query.build_query_as().fetch_all(pool).await
}

async fn error_summary(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<Vec<ErrorSummaryRow>> {
    let request_filters = request_filters_from_usage(filters, None);
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT COALESCE(error_code, 'http_' || COALESCE(status_code, 500)) AS error_code,
                status_code,
                COUNT(*) AS count,
                MAX(started_at) AS last_seen_at
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, &request_filters);
    query.push(if request_log_filters_empty(&request_filters) {
        " WHERE "
    } else {
        " AND "
    });
    query.push(
        "COALESCE(status_code, 500) >= 400
         GROUP BY COALESCE(error_code, 'http_' || COALESCE(status_code, 500)), status_code
         ORDER BY count DESC, last_seen_at DESC
         LIMIT 20",
    );
    query.build_query_as().fetch_all(pool).await
}

fn request_filters_from_usage(
    filters: &DailyUsageFilters,
    limit: Option<i64>,
) -> RequestLogFilters {
    RequestLogFilters {
        user_id: filters.user_id.clone(),
        api_key_id: filters.api_key_id.clone(),
        model_id: filters.model_id.clone(),
        upstream_id: filters.upstream_id.clone(),
        status_code: None,
        started_at_from: filters
            .date_from
            .as_ref()
            .map(|date| format!("{date}T00:00:00.000Z")),
        started_at_to: filters
            .date_to
            .as_ref()
            .map(|date| format!("{date}T23:59:59.999Z")),
        limit,
    }
}

fn push_request_log_filter_where<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    filters: &'a RequestLogFilters,
) {
    let mut has_where = false;
    if let Some(user_id) = &filters.user_id {
        push_where(query, &mut has_where);
        query.push("user_id = ").push_bind(user_id);
    }
    if let Some(api_key_id) = &filters.api_key_id {
        push_where(query, &mut has_where);
        query.push("api_key_id = ").push_bind(api_key_id);
    }
    if let Some(model_id) = &filters.model_id {
        push_where(query, &mut has_where);
        query.push("model_id = ").push_bind(model_id);
    }
    if let Some(upstream_id) = &filters.upstream_id {
        push_where(query, &mut has_where);
        query.push("upstream_id = ").push_bind(upstream_id);
    }
    if let Some(status_code) = filters.status_code {
        push_where(query, &mut has_where);
        query.push("status_code = ").push_bind(status_code);
    }
    if let Some(started_at_from) = &filters.started_at_from {
        push_where(query, &mut has_where);
        query.push("started_at >= ").push_bind(started_at_from);
    }
    if let Some(started_at_to) = &filters.started_at_to {
        push_where(query, &mut has_where);
        query.push("started_at <= ").push_bind(started_at_to);
    }
}

fn request_log_filters_empty(filters: &RequestLogFilters) -> bool {
    filters.user_id.is_none()
        && filters.api_key_id.is_none()
        && filters.model_id.is_none()
        && filters.upstream_id.is_none()
        && filters.status_code.is_none()
        && filters.started_at_from.is_none()
        && filters.started_at_to.is_none()
}

pub async fn gateway_metrics(pool: &SqlitePool) -> sqlx::Result<GatewayMetrics> {
    let totals: (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT
                SUM(request_count),
                SUM(error_count),
                SUM(latency_ms_sum),
                SUM(prompt_tokens),
                SUM(completion_tokens),
                SUM(total_tokens)
             FROM daily_usage",
    )
    .fetch_one(pool)
    .await?;
    let request_count = totals.0.unwrap_or_default();
    let latency_sum = totals.2.unwrap_or_default();
    let upstream_health = sqlx::query_as(
        "SELECT
            upstreams.id AS upstream_id,
            upstreams.name AS name,
            upstreams.enabled AS enabled,
            upstreams.last_health_status AS last_health_status,
            upstreams.last_health_checked_at AS last_health_checked_at,
            upstreams.last_degraded_at AS last_degraded_at,
            upstreams.last_down_at AS last_down_at,
            upstreams.recent_error_samples AS recent_error_samples,
            COALESCE(SUM(daily_usage.request_count), 0) AS request_count,
            COALESCE(SUM(daily_usage.error_count), 0) AS error_count,
            COALESCE(SUM(daily_usage.latency_ms_sum), 0) AS latency_ms_sum,
            COALESCE(SUM(daily_usage.total_tokens), 0) AS total_tokens
         FROM upstreams
         LEFT JOIN daily_usage ON daily_usage.upstream_id = upstreams.id
         GROUP BY upstreams.id
         ORDER BY upstreams.enabled DESC,
                  CASE upstreams.last_health_status
                    WHEN 'down' THEN 0
                    WHEN 'degraded' THEN 1
                    WHEN 'unknown' THEN 2
                    ELSE 3
                  END,
                  upstreams.priority,
                  upstreams.name",
    )
    .fetch_all(pool)
    .await?;

    Ok(GatewayMetrics {
        generated_at: now_string(),
        request_count,
        error_count: totals.1.unwrap_or_default(),
        latency: LatencyMetrics {
            sum_ms: latency_sum,
            avg_ms: (request_count > 0).then_some(latency_sum as f64 / request_count as f64),
        },
        token_usage: TokenUsageMetrics {
            prompt_tokens: totals.3.unwrap_or_default(),
            completion_tokens: totals.4.unwrap_or_default(),
            total_tokens: totals.5.unwrap_or_default(),
        },
        upstream_health,
    })
}

pub async fn apply_retention(
    pool: &SqlitePool,
    policy: &RetentionPolicy,
) -> sqlx::Result<RetentionResult> {
    apply_retention_at(pool, policy, Utc::now()).await
}

pub async fn apply_retention_at(
    pool: &SqlitePool,
    policy: &RetentionPolicy,
    now: DateTime<Utc>,
) -> sqlx::Result<RetentionResult> {
    let request_log_cutoff = (policy.request_log_retention_days > 0).then(|| {
        (now - Duration::days(policy.request_log_retention_days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    });
    let daily_usage_cutoff = (policy.daily_usage_retention_days > 0).then(|| {
        (now - Duration::days(policy.daily_usage_retention_days))
            .date_naive()
            .to_string()
    });

    let request_logs_deleted = if let Some(cutoff) = &request_log_cutoff {
        sqlx::query("DELETE FROM request_logs WHERE started_at < ?")
            .bind(cutoff)
            .execute(pool)
            .await?
            .rows_affected()
    } else {
        0
    };
    let daily_usage_deleted = if let Some(cutoff) = &daily_usage_cutoff {
        sqlx::query("DELETE FROM daily_usage WHERE date < ?")
            .bind(cutoff)
            .execute(pool)
            .await?
            .rows_affected()
    } else {
        0
    };

    Ok(RetentionResult {
        request_logs_deleted,
        daily_usage_deleted,
        request_log_cutoff,
        daily_usage_cutoff,
    })
}

pub async fn insert_request_log(pool: &SqlitePool, log: RequestLogInsert) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let id = auth::new_id();
    sqlx::query(
        "INSERT INTO request_logs
         (id, request_id, user_id, api_key_id, model_id, upstream_id, method, path, status_code, error_code, stream,
          prompt_tokens, completion_tokens, total_tokens, usage_source, input_chars, output_chars, latency_ms,
          started_at, finished_at, client_ip_hash, user_agent, upstream_response_id, upstream_status, client_metadata_sanitized,
          route_strategy, route_decision_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&log.request_id)
    .bind(&log.user_id)
    .bind(&log.api_key_id)
    .bind(&log.model_id)
    .bind(&log.upstream_id)
    .bind(&log.method)
    .bind(&log.path)
    .bind(log.status_code)
    .bind(&log.error_code)
    .bind(bool_to_i64(log.stream))
    .bind(log.usage.prompt_tokens)
    .bind(log.usage.completion_tokens)
    .bind(log.usage.total_tokens)
    .bind(log.usage.source.as_str())
    .bind(log.input_chars)
    .bind(log.output_chars)
    .bind(log.latency_ms)
    .bind(&log.started_at)
    .bind(&log.finished_at)
    .bind(&log.client_ip_hash)
    .bind(&log.user_agent)
    .bind(&log.usage.upstream_response_id)
    .bind(&log.usage.upstream_status)
    .bind(&log.client_metadata_sanitized)
    .bind(&log.route_strategy)
    .bind(&log.route_decision_json)
    .execute(&mut *tx)
    .await?;

    upsert_daily_usage(&mut tx, &log).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn insert_admin_audit_log(pool: &SqlitePool, log: AdminAuditInsert) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO admin_audit_logs
         (id, actor_user_id, actor_email, action, resource_type, resource_id, status, metadata_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(auth::new_id())
    .bind(log.actor_user_id)
    .bind(log.actor_email)
    .bind(log.action)
    .bind(log.resource_type)
    .bind(log.resource_id)
    .bind(log.status)
    .bind(log.metadata_json)
    .bind(now_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_limit_policy(
    pool: &SqlitePool,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<Option<LimitPolicy>> {
    sqlx::query_as(
        "SELECT scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
                token_quota, token_quota_mode, token_window_seconds,
                rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
                concurrency_limit, concurrency_mode, created_at, updated_at
         FROM limit_policies
         WHERE scope = ? AND subject_id = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_limit_policy(
    pool: &SqlitePool,
    scope: &str,
    subject_id: &str,
    patch: &LimitPolicyPatch,
) -> sqlx::Result<LimitPolicy> {
    let existing = get_limit_policy(pool, scope, subject_id).await?;
    let base = existing.unwrap_or_else(|| default_policy(scope, subject_id));
    let now = now_string();
    let request_quota = apply_nullable_limit_patch(
        &patch.request_quota,
        base.request_quota,
        &base.request_quota_mode,
        scope,
    );
    let token_quota = apply_nullable_limit_patch(
        &patch.token_quota,
        base.token_quota,
        &base.token_quota_mode,
        scope,
    );
    let rate_limit = apply_nullable_limit_patch(
        &patch.rate_limit_requests,
        base.rate_limit_requests,
        &base.rate_limit_mode,
        scope,
    );
    let concurrency = apply_nullable_limit_patch(
        &patch.concurrency_limit,
        base.concurrency_limit,
        &base.concurrency_mode,
        scope,
    );
    let policy = LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: request_quota.0,
        request_quota_mode: request_quota.1,
        request_window_seconds: patch
            .request_window_seconds
            .unwrap_or(base.request_window_seconds),
        token_quota: token_quota.0,
        token_quota_mode: token_quota.1,
        token_window_seconds: patch
            .token_window_seconds
            .unwrap_or(base.token_window_seconds),
        rate_limit_requests: rate_limit.0,
        rate_limit_mode: rate_limit.1,
        rate_limit_window_seconds: patch
            .rate_limit_window_seconds
            .unwrap_or(base.rate_limit_window_seconds),
        concurrency_limit: concurrency.0,
        concurrency_mode: concurrency.1,
        created_at: base.created_at,
        updated_at: now.clone(),
    };
    sqlx::query(
        "INSERT INTO limit_policies
         (scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
          token_quota, token_quota_mode, token_window_seconds,
          rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
          concurrency_limit, concurrency_mode, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(scope, subject_id) DO UPDATE SET
            request_quota = excluded.request_quota,
            request_quota_mode = excluded.request_quota_mode,
            request_window_seconds = excluded.request_window_seconds,
            token_quota = excluded.token_quota,
            token_quota_mode = excluded.token_quota_mode,
            token_window_seconds = excluded.token_window_seconds,
            rate_limit_requests = excluded.rate_limit_requests,
            rate_limit_mode = excluded.rate_limit_mode,
            rate_limit_window_seconds = excluded.rate_limit_window_seconds,
            concurrency_limit = excluded.concurrency_limit,
            concurrency_mode = excluded.concurrency_mode,
            updated_at = excluded.updated_at",
    )
    .bind(&policy.scope)
    .bind(&policy.subject_id)
    .bind(policy.request_quota)
    .bind(&policy.request_quota_mode)
    .bind(policy.request_window_seconds)
    .bind(policy.token_quota)
    .bind(&policy.token_quota_mode)
    .bind(policy.token_window_seconds)
    .bind(policy.rate_limit_requests)
    .bind(&policy.rate_limit_mode)
    .bind(policy.rate_limit_window_seconds)
    .bind(policy.concurrency_limit)
    .bind(&policy.concurrency_mode)
    .bind(&policy.created_at)
    .bind(&policy.updated_at)
    .execute(pool)
    .await?;
    get_limit_policy(pool, scope, subject_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn user_limit_state(
    pool: &SqlitePool,
    user_id: &str,
    current_api_key_id: Option<&str>,
) -> sqlx::Result<UserLimitState> {
    let system = system_limit_policy(pool).await?;
    let user_stored_policy = get_limit_policy(pool, "user", user_id).await?;
    let user_policy = merge_policy(&system, user_stored_policy.as_ref(), "user", user_id);
    let user_display_policy = display_policy(user_stored_policy, &user_policy, "user", user_id);
    let user = limit_subject_state(
        pool,
        user_id,
        "user",
        user_id,
        user_display_policy,
        user_policy,
    )
    .await?;
    let keys = list_api_keys_for_user(pool, user_id).await?;
    let mut api_keys = Vec::with_capacity(keys.len());
    let mut current_key = None;
    for key in keys {
        let state = api_key_limit_state(pool, &system, &key.id).await?;
        if current_api_key_id == Some(key.id.as_str()) {
            current_key = Some(state.clone());
        }
        api_keys.push(state);
    }
    Ok(UserLimitState {
        user,
        current_key,
        api_keys,
    })
}

pub async fn admin_limit_state(pool: &SqlitePool) -> sqlx::Result<AdminLimitState> {
    let system = system_limit_policy(pool).await?;
    let users = list_users(pool).await?;
    let keys = list_api_keys(pool).await?;
    let mut user_states = Vec::with_capacity(users.len());
    for user in users {
        let stored_policy = get_limit_policy(pool, "user", &user.id).await?;
        let policy = merge_policy(&system, stored_policy.as_ref(), "user", &user.id);
        let display = display_policy(stored_policy, &policy, "user", &user.id);
        user_states
            .push(limit_subject_state(pool, &user.id, "user", &user.id, display, policy).await?);
    }
    let mut key_states = Vec::with_capacity(keys.len());
    for key in keys {
        key_states.push(api_key_limit_state(pool, &system, &key.id).await?);
    }
    Ok(AdminLimitState {
        system,
        users: user_states,
        api_keys: key_states,
    })
}

pub async fn admit_limited_request(
    pool: &SqlitePool,
    user_id: &str,
    api_key_id: &str,
) -> Result<LimitAdmission, LimitAdmissionError> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let result = admit_limited_request_in_tx(&mut conn, user_id, api_key_id).await;
    match result {
        Ok(admission) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok(admission)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(error)
        }
    }
}

async fn admit_limited_request_in_tx(
    conn: &mut sqlx::SqliteConnection,
    user_id: &str,
    api_key_id: &str,
) -> Result<LimitAdmission, LimitAdmissionError> {
    let now = Utc::now();
    let now_text = timestamp(now);
    sqlx::query("DELETE FROM limit_inflight_requests WHERE expires_at <= ?")
        .bind(&now_text)
        .execute(&mut *conn)
        .await?;

    let system = system_limit_policy_conn(conn).await?;
    let user_policy = effective_subject_policy_conn(conn, &system, "user", user_id).await?;
    let key_policy = effective_subject_policy_conn(conn, &system, "api_key", api_key_id).await?;
    let scopes = vec![
        EnforcedLimitScope {
            scope: "user",
            subject_id: user_id,
            policy: user_policy,
        },
        EnforcedLimitScope {
            scope: "api_key",
            subject_id: api_key_id,
            policy: key_policy,
        },
    ];

    for scope in &scopes {
        if let Some(rejection) = limit_rejection_for_scope(conn, scope, now).await? {
            return Err(LimitAdmissionError::Rejected(rejection));
        }
    }

    let usage_event_id = auth::new_id();
    sqlx::query(
        "INSERT INTO limit_usage_events
         (id, user_id, api_key_id, request_count, total_tokens, created_at)
         VALUES (?, ?, ?, 1, 0, ?)",
    )
    .bind(&usage_event_id)
    .bind(user_id)
    .bind(api_key_id)
    .bind(&now_text)
    .execute(&mut *conn)
    .await?;

    let inflight_request_id = auth::new_id();
    let expires_at = timestamp(now + Duration::hours(6));
    sqlx::query(
        "INSERT INTO limit_inflight_requests (id, user_id, api_key_id, started_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&inflight_request_id)
    .bind(user_id)
    .bind(api_key_id)
    .bind(&now_text)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;

    for scope in &scopes {
        let window_started_at = rate_window_start(now, scope.policy.rate_limit_window_seconds);
        sqlx::query(
            "INSERT INTO limit_rate_counters
             (scope, subject_id, window_started_at, request_count, updated_at)
             VALUES (?, ?, ?, 1, ?)
             ON CONFLICT(scope, subject_id, window_started_at) DO UPDATE SET
                request_count = request_count + 1,
                updated_at = excluded.updated_at",
        )
        .bind(scope.scope)
        .bind(scope.subject_id)
        .bind(window_started_at)
        .bind(&now_text)
        .execute(&mut *conn)
        .await?;
    }

    Ok(LimitAdmission {
        usage_event_id,
        inflight_request_id,
    })
}

pub async fn finalize_limit_admission(
    pool: &SqlitePool,
    admission: &LimitAdmission,
    total_tokens: i64,
) -> sqlx::Result<()> {
    let now = now_string();
    sqlx::query(
        "UPDATE limit_usage_events
         SET total_tokens = ?, finalized_at = ?
         WHERE id = ?",
    )
    .bind(total_tokens.max(0))
    .bind(&now)
    .bind(&admission.usage_event_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM limit_inflight_requests WHERE id = ?")
        .bind(&admission.inflight_request_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn api_key_limit_state(
    pool: &SqlitePool,
    system: &LimitPolicy,
    api_key_id: &str,
) -> sqlx::Result<LimitSubjectState> {
    let stored_policy = get_limit_policy(pool, "api_key", api_key_id).await?;
    let policy = merge_policy(system, stored_policy.as_ref(), "api_key", api_key_id);
    let display = display_policy(stored_policy, &policy, "api_key", api_key_id);
    limit_subject_state(pool, api_key_id, "api_key", api_key_id, display, policy).await
}

async fn system_limit_policy(pool: &SqlitePool) -> sqlx::Result<LimitPolicy> {
    if let Some(policy) = get_limit_policy(pool, "system", "").await? {
        return Ok(policy);
    }
    upsert_limit_policy(pool, "system", "", &LimitPolicyPatch::default()).await
}

async fn system_limit_policy_conn(conn: &mut sqlx::SqliteConnection) -> sqlx::Result<LimitPolicy> {
    get_limit_policy_conn(conn, "system", "")
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

async fn effective_subject_policy_conn(
    conn: &mut sqlx::SqliteConnection,
    system: &LimitPolicy,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<LimitPolicy> {
    let override_policy = get_limit_policy_conn(conn, scope, subject_id).await?;
    Ok(merge_policy(
        system,
        override_policy.as_ref(),
        scope,
        subject_id,
    ))
}

async fn get_limit_policy_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<Option<LimitPolicy>> {
    sqlx::query_as(
        "SELECT scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
                token_quota, token_quota_mode, token_window_seconds,
                rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
                concurrency_limit, concurrency_mode, created_at, updated_at
         FROM limit_policies
         WHERE scope = ? AND subject_id = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .fetch_optional(&mut *conn)
    .await
}

fn merge_policy(
    system: &LimitPolicy,
    override_policy: Option<&LimitPolicy>,
    scope: &str,
    subject_id: &str,
) -> LimitPolicy {
    let Some(override_policy) = override_policy else {
        let mut policy = system.clone();
        policy.scope = scope.to_string();
        policy.subject_id = subject_id.to_string();
        return policy;
    };
    let request_quota = resolve_nullable_limit(
        system.request_quota,
        &system.request_quota_mode,
        override_policy.request_quota,
        &override_policy.request_quota_mode,
    );
    let token_quota = resolve_nullable_limit(
        system.token_quota,
        &system.token_quota_mode,
        override_policy.token_quota,
        &override_policy.token_quota_mode,
    );
    let rate_limit = resolve_nullable_limit(
        system.rate_limit_requests,
        &system.rate_limit_mode,
        override_policy.rate_limit_requests,
        &override_policy.rate_limit_mode,
    );
    let concurrency = resolve_nullable_limit(
        system.concurrency_limit,
        &system.concurrency_mode,
        override_policy.concurrency_limit,
        &override_policy.concurrency_mode,
    );
    LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: request_quota.0,
        request_quota_mode: request_quota.1,
        request_window_seconds: inherited_window_seconds(
            system.request_window_seconds,
            override_policy.request_window_seconds,
            &override_policy.request_quota_mode,
        ),
        token_quota: token_quota.0,
        token_quota_mode: token_quota.1,
        token_window_seconds: inherited_window_seconds(
            system.token_window_seconds,
            override_policy.token_window_seconds,
            &override_policy.token_quota_mode,
        ),
        rate_limit_requests: rate_limit.0,
        rate_limit_mode: rate_limit.1,
        rate_limit_window_seconds: inherited_window_seconds(
            system.rate_limit_window_seconds,
            override_policy.rate_limit_window_seconds,
            &override_policy.rate_limit_mode,
        ),
        concurrency_limit: concurrency.0,
        concurrency_mode: concurrency.1,
        created_at: override_policy.created_at.clone(),
        updated_at: override_policy.updated_at.clone(),
    }
}

fn display_policy(
    stored_policy: Option<LimitPolicy>,
    effective_policy: &LimitPolicy,
    scope: &str,
    subject_id: &str,
) -> LimitPolicy {
    let mut policy = stored_policy.unwrap_or_else(|| default_policy(scope, subject_id));
    policy.request_window_seconds = inherited_window_seconds(
        effective_policy.request_window_seconds,
        policy.request_window_seconds,
        &policy.request_quota_mode,
    );
    policy.token_window_seconds = inherited_window_seconds(
        effective_policy.token_window_seconds,
        policy.token_window_seconds,
        &policy.token_quota_mode,
    );
    policy.rate_limit_window_seconds = inherited_window_seconds(
        effective_policy.rate_limit_window_seconds,
        policy.rate_limit_window_seconds,
        &policy.rate_limit_mode,
    );
    policy
}

fn inherited_window_seconds(system_window: i64, override_window: i64, mode: &str) -> i64 {
    if mode == "inherit" {
        system_window
    } else {
        override_window
    }
}

fn apply_nullable_limit_patch(
    patch: &LimitPatchValue,
    current_value: Option<i64>,
    current_mode: &str,
    scope: &str,
) -> (Option<i64>, String) {
    match patch {
        LimitPatchValue::Missing => (current_value, current_mode.to_string()),
        LimitPatchValue::Inherit => (None, "inherit".to_string()),
        LimitPatchValue::Clear => (None, "unlimited".to_string()),
        LimitPatchValue::Set(value) => (Some(*value), "limited".to_string()),
    }
    .normalize_system_mode(scope)
}

fn resolve_nullable_limit(
    system_value: Option<i64>,
    system_mode: &str,
    override_value: Option<i64>,
    override_mode: &str,
) -> (Option<i64>, String) {
    match override_mode {
        "inherit" => match system_mode {
            "limited" => (system_value, "limited".to_string()),
            _ => (None, "unlimited".to_string()),
        },
        "limited" => (override_value, "limited".to_string()),
        "unlimited" => (None, "unlimited".to_string()),
        _ => (override_value.or(system_value), "limited".to_string()),
    }
}

trait LimitModeNormalize {
    fn normalize_system_mode(self, scope: &str) -> Self;
}

impl LimitModeNormalize for (Option<i64>, String) {
    fn normalize_system_mode(self, scope: &str) -> Self {
        if scope != "system" || self.1 != "inherit" {
            return self;
        }
        if self.0.is_some() {
            (self.0, "limited".to_string())
        } else {
            (None, "unlimited".to_string())
        }
    }
}

struct EnforcedLimitScope<'a> {
    scope: &'static str,
    subject_id: &'a str,
    policy: LimitPolicy,
}

async fn limit_rejection_for_scope(
    conn: &mut sqlx::SqliteConnection,
    scope: &EnforcedLimitScope<'_>,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<LimitRejection>> {
    if let Some(limit) = scope.policy.request_quota {
        let used = usage_count_conn(
            conn,
            scope.scope,
            scope.subject_id,
            "request_count",
            now,
            scope.policy.request_window_seconds,
        )
        .await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "quota_exceeded",
                message: "request quota exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "request_quota",
                limit,
                used,
                reset_at: Some(timestamp(
                    now + Duration::seconds(scope.policy.request_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.token_quota {
        let used = usage_count_conn(
            conn,
            scope.scope,
            scope.subject_id,
            "total_tokens",
            now,
            scope.policy.token_window_seconds,
        )
        .await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "quota_exceeded",
                message: "token budget exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "token_budget",
                limit,
                used,
                reset_at: Some(timestamp(
                    now + Duration::seconds(scope.policy.token_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.rate_limit_requests {
        let window_started_at = rate_window_start(now, scope.policy.rate_limit_window_seconds);
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(request_count, 0)
             FROM limit_rate_counters
             WHERE scope = ? AND subject_id = ? AND window_started_at = ?",
        )
        .bind(scope.scope)
        .bind(scope.subject_id)
        .bind(&window_started_at)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or_default();
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "rate_limited",
                message: "rate limit exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "rate_limit",
                limit,
                used,
                reset_at: Some(timestamp(
                    parse_timestamp(&window_started_at)
                        + Duration::seconds(scope.policy.rate_limit_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.concurrency_limit {
        let used = inflight_count_conn(conn, scope.scope, scope.subject_id).await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "concurrency_limited",
                message: "concurrent request limit exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "concurrency",
                limit,
                used,
                reset_at: None,
            }));
        }
    }
    Ok(None)
}

async fn limit_subject_state(
    pool: &SqlitePool,
    owner_user_id: &str,
    scope: &str,
    subject_id: &str,
    display_policy: LimitPolicy,
    policy: LimitPolicy,
) -> sqlx::Result<LimitSubjectState> {
    let now = Utc::now();
    let request_used = usage_count(
        pool,
        scope,
        owner_user_id,
        subject_id,
        "request_count",
        now,
        policy.request_window_seconds,
    )
    .await?;
    let token_used = usage_count(
        pool,
        scope,
        owner_user_id,
        subject_id,
        "total_tokens",
        now,
        policy.token_window_seconds,
    )
    .await?;
    let rate_window_started_at = rate_window_start(now, policy.rate_limit_window_seconds);
    let rate_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(request_count, 0)
         FROM limit_rate_counters
         WHERE scope = ? AND subject_id = ? AND window_started_at = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .bind(&rate_window_started_at)
    .fetch_optional(pool)
    .await?
    .unwrap_or_default();
    let in_flight = inflight_count(pool, scope, subject_id).await?;

    Ok(LimitSubjectState {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        effective_policy: policy.clone(),
        request_quota: bucket_state(
            policy.request_quota,
            request_used,
            Some(policy.request_window_seconds),
            Some(timestamp(
                now + Duration::seconds(policy.request_window_seconds),
            )),
        ),
        token_budget: bucket_state(
            policy.token_quota,
            token_used,
            Some(policy.token_window_seconds),
            Some(timestamp(
                now + Duration::seconds(policy.token_window_seconds),
            )),
        ),
        rate_limit: bucket_state(
            policy.rate_limit_requests,
            rate_used,
            Some(policy.rate_limit_window_seconds),
            Some(timestamp(
                parse_timestamp(&rate_window_started_at)
                    + Duration::seconds(policy.rate_limit_window_seconds),
            )),
        ),
        concurrency: ConcurrencyState {
            limit: policy.concurrency_limit,
            in_flight,
            remaining: remaining(policy.concurrency_limit, in_flight),
        },
        policy: display_policy,
    })
}

async fn usage_count(
    pool: &SqlitePool,
    scope: &str,
    owner_user_id: &str,
    subject_id: &str,
    column: &str,
    now: DateTime<Utc>,
    window_seconds: i64,
) -> sqlx::Result<i64> {
    let cutoff = timestamp(now - Duration::seconds(window_seconds.max(1)));
    let sql = match (scope, column) {
        ("user", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("user", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("api_key", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        ("api_key", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        _ => return Ok(0),
    };
    let id = if scope == "user" {
        owner_user_id
    } else {
        subject_id
    };
    sqlx::query_scalar(sql)
        .bind(id)
        .bind(cutoff)
        .fetch_one(pool)
        .await
}

async fn usage_count_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
    column: &str,
    now: DateTime<Utc>,
    window_seconds: i64,
) -> sqlx::Result<i64> {
    let cutoff = timestamp(now - Duration::seconds(window_seconds.max(1)));
    let sql = match (scope, column) {
        ("user", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("user", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("api_key", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        ("api_key", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        _ => return Ok(0),
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .bind(cutoff)
        .fetch_one(&mut *conn)
        .await
}

async fn inflight_count(pool: &SqlitePool, scope: &str, subject_id: &str) -> sqlx::Result<i64> {
    let sql = if scope == "api_key" {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE api_key_id = ?"
    } else {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE user_id = ?"
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .fetch_one(pool)
        .await
}

async fn inflight_count_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<i64> {
    let sql = if scope == "api_key" {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE api_key_id = ?"
    } else {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE user_id = ?"
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .fetch_one(&mut *conn)
        .await
}

fn bucket_state(
    limit: Option<i64>,
    used: i64,
    window_seconds: Option<i64>,
    reset_at: Option<String>,
) -> LimitBucketState {
    LimitBucketState {
        limit,
        used,
        remaining: remaining(limit, used),
        window_seconds,
        reset_at,
    }
}

fn remaining(limit: Option<i64>, used: i64) -> Option<i64> {
    limit.map(|limit| (limit - used).max(0))
}

fn default_policy(scope: &str, subject_id: &str) -> LimitPolicy {
    let now = now_string();
    let nullable_mode = if scope == "system" {
        "unlimited"
    } else {
        "inherit"
    };
    LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: None,
        request_quota_mode: nullable_mode.to_string(),
        request_window_seconds: 86_400,
        token_quota: None,
        token_quota_mode: nullable_mode.to_string(),
        token_window_seconds: 86_400,
        rate_limit_requests: None,
        rate_limit_mode: nullable_mode.to_string(),
        rate_limit_window_seconds: 60,
        concurrency_limit: None,
        concurrency_mode: nullable_mode.to_string(),
        created_at: now.clone(),
        updated_at: now,
    }
}

async fn clear_stale_limit_inflight(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM limit_inflight_requests")
        .execute(pool)
        .await?;
    Ok(())
}

fn rate_window_start(now: DateTime<Utc>, window_seconds: i64) -> String {
    let window_seconds = window_seconds.max(1);
    let timestamp = now.timestamp();
    let start = timestamp - timestamp.rem_euclid(window_seconds);
    DateTime::<Utc>::from_timestamp(start, 0)
        .unwrap_or(now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub async fn list_admin_audit_logs(pool: &SqlitePool) -> sqlx::Result<Vec<AdminAuditLog>> {
    sqlx::query_as(
        "SELECT id, actor_user_id, actor_email, action, resource_type, resource_id, status, metadata_json, created_at
         FROM admin_audit_logs
         ORDER BY created_at DESC
         LIMIT 500",
    )
    .fetch_all(pool)
    .await
}

async fn upsert_daily_usage(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    log: &RequestLogInsert,
) -> sqlx::Result<()> {
    let day = log.started_at.get(0..10).unwrap_or("unknown");
    let error_count = i64::from(log.status_code.unwrap_or(500) >= 400);
    let stream_count = i64::from(log.stream);
    sqlx::query(
        "INSERT INTO daily_usage
         (id, date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count,
          prompt_tokens, completion_tokens, total_tokens, latency_ms_sum, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(date, user_id, api_key_id, model_id, upstream_id) DO UPDATE SET
            request_count = request_count + 1,
            error_count = error_count + excluded.error_count,
            stream_count = stream_count + excluded.stream_count,
            prompt_tokens = prompt_tokens + excluded.prompt_tokens,
            completion_tokens = completion_tokens + excluded.completion_tokens,
            total_tokens = total_tokens + excluded.total_tokens,
            latency_ms_sum = latency_ms_sum + excluded.latency_ms_sum,
            updated_at = excluded.updated_at",
    )
    .bind(auth::new_id())
    .bind(day)
    .bind(&log.user_id)
    .bind(&log.api_key_id)
    .bind(&log.model_id)
    .bind(&log.upstream_id)
    .bind(error_count)
    .bind(stream_count)
    .bind(log.usage.prompt_tokens)
    .bind(log.usage.completion_tokens)
    .bind(log.usage.total_tokens)
    .bind(log.latency_ms)
    .bind(now_string())
    .bind(now_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub fn now_string() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}
