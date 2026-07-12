use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::{auth, config::Config};

use super::db::{bool_to_i64, now_string};

type UpstreamHealthSnapshot = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

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

#[derive(Clone, FromRow)]
pub struct UpstreamRecord {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key_ciphertext: String,
    pub api_key_secret_version: i64,
    pub enabled: i64,
    pub priority: i64,
    pub weight: i64,
    pub timeout_ms: i64,
    pub timeout_ms_is_explicit: i64,
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

pub type Upstream = UpstreamRecord;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TimeoutPatchValue {
    #[default]
    Missing,
    Default,
    Explicit(i64),
}

impl TimeoutPatchValue {
    pub fn explicit_value(&self) -> Option<i64> {
        match self {
            Self::Explicit(value) => Some(*value),
            Self::Missing | Self::Default => None,
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

#[derive(Clone, Debug)]
pub struct UpsertUpstream {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub timeout_ms: TimeoutPatchValue,
    pub max_retries: Option<i64>,
    pub health_check_path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UpdateUpstream {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub timeout_ms: TimeoutPatchValue,
    pub max_retries: Option<i64>,
    pub health_check_path: Option<String>,
}

pub async fn list_upstreams(pool: &SqlitePool) -> sqlx::Result<Vec<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, timeout_ms_is_explicit, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams ORDER BY priority, name")
        .fetch_all(pool)
        .await
}

pub async fn list_enabled_upstreams(pool: &SqlitePool) -> sqlx::Result<Vec<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, timeout_ms_is_explicit, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams WHERE enabled = 1 ORDER BY priority, name")
        .fetch_all(pool)
        .await
}

pub async fn create_upstream(
    pool: &SqlitePool,
    app_secret: &str,
    secret_key_version: i64,
    input: &UpsertUpstream,
) -> anyhow::Result<Upstream> {
    let mut conn = pool.acquire().await?;
    create_upstream_conn(&mut conn, app_secret, secret_key_version, input).await
}

pub async fn create_upstream_conn(
    conn: &mut sqlx::SqliteConnection,
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
         (id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, timeout_ms_is_explicit, max_retries, health_check_path, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(input.base_url.trim_end_matches('/'))
    .bind(encrypted_key)
    .bind(secret_key_version)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(input.priority.unwrap_or(100))
    .bind(input.weight.unwrap_or(1).max(1))
    .bind(
        input
            .timeout_ms
            .explicit_value()
            .unwrap_or_else(crate::config::default_request_timeout_ms),
    )
    .bind(bool_to_i64(matches!(
        input.timeout_ms,
        TimeoutPatchValue::Explicit(_)
    )))
    .bind(input.max_retries.unwrap_or(1))
    .bind(input.health_check_path.as_deref().unwrap_or("/v1/models"))
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    get_upstream_conn(conn, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created upstream not found"))
}

pub async fn get_upstream(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, timeout_ms_is_explicit, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn get_upstream_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> sqlx::Result<Option<Upstream>> {
    sqlx::query_as("SELECT id, name, base_url, api_key_ciphertext, api_key_secret_version, enabled, priority, weight, timeout_ms, timeout_ms_is_explicit, max_retries, health_check_path, last_health_status, last_health_checked_at, health_status_changed_at, last_degraded_at, last_down_at, recent_error_samples, created_at, updated_at FROM upstreams WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

pub async fn update_upstream(
    pool: &SqlitePool,
    app_secret: &str,
    secret_key_version: i64,
    id: &str,
    input: &UpdateUpstream,
) -> anyhow::Result<Option<Upstream>> {
    let mut conn = pool.acquire().await?;
    update_upstream_conn(&mut conn, app_secret, secret_key_version, id, input).await
}

pub async fn update_upstream_conn(
    conn: &mut sqlx::SqliteConnection,
    app_secret: &str,
    secret_key_version: i64,
    id: &str,
    input: &UpdateUpstream,
) -> anyhow::Result<Option<Upstream>> {
    let Some(existing) = get_upstream_conn(conn, id).await? else {
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
    let (timeout_ms, timeout_ms_is_explicit) = match input.timeout_ms {
        TimeoutPatchValue::Missing => (existing.timeout_ms, existing.timeout_ms_is_explicit),
        TimeoutPatchValue::Default => (existing.timeout_ms, 0),
        TimeoutPatchValue::Explicit(value) => (value, 1),
    };
    let max_retries = input.max_retries.unwrap_or(existing.max_retries);
    let health_check_path = input
        .health_check_path
        .as_deref()
        .unwrap_or(&existing.health_check_path);
    let now = now_string();
    sqlx::query(
        "UPDATE upstreams
         SET name = ?, base_url = ?, api_key_ciphertext = ?, api_key_secret_version = ?, enabled = ?,
             priority = ?, weight = ?, timeout_ms = ?, timeout_ms_is_explicit = ?, max_retries = ?,
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
    .bind(timeout_ms_is_explicit)
    .bind(max_retries)
    .bind(health_check_path)
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(get_upstream_conn(conn, id).await?)
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
    let mut conn = pool.acquire().await?;
    record_upstream_health_conn(&mut conn, id, status, error_sample).await
}

pub async fn record_upstream_health_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    status: &str,
    error_sample: Option<&str>,
) -> sqlx::Result<()> {
    let existing: Option<UpstreamHealthSnapshot> = sqlx::query_as(
        "SELECT last_health_status, recent_error_samples, health_status_changed_at, last_degraded_at, last_down_at
         FROM upstreams
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
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
    .execute(&mut *conn)
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
