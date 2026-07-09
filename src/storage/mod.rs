use std::{path::Path, str::FromStr};

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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
    if let Some(user_id) = user_id {
        sqlx::query_as("SELECT date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count, prompt_tokens, completion_tokens, total_tokens, latency_ms_sum FROM daily_usage WHERE user_id = ? ORDER BY date DESC LIMIT 90")
            .bind(user_id)
            .fetch_all(pool)
            .await
    } else {
        sqlx::query_as("SELECT date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count, prompt_tokens, completion_tokens, total_tokens, latency_ms_sum FROM daily_usage ORDER BY date DESC LIMIT 500")
            .fetch_all(pool)
            .await
    }
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
    .execute(pool)
    .await?;

    upsert_daily_usage(pool, &log).await?;
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

async fn upsert_daily_usage(pool: &SqlitePool, log: &RequestLogInsert) -> sqlx::Result<()> {
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
    .execute(pool)
    .await?;
    Ok(())
}

pub fn now_string() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}
