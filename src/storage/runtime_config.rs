use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::config::{
    Config, DEFAULT_DAILY_USAGE_RETENTION_DAYS, DEFAULT_EXPOSE_DEBUG_HEADERS,
    DEFAULT_MAX_REQUEST_BODY_BYTES, DEFAULT_REQUEST_LOG_RETENTION_DAYS, DEFAULT_REQUEST_TIMEOUT_MS,
    DEFAULT_ROUTE_STRATEGY, RouteStrategy, RuntimeConfig,
};

use super::db::now_string;

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct SystemConfig {
    pub route_strategy: Option<String>,
    pub default_request_timeout_ms: Option<i64>,
    pub max_request_body_bytes: Option<i64>,
    pub request_log_retention_days: Option<i64>,
    pub daily_usage_retention_days: Option<i64>,
    pub expose_debug_headers: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct SystemConfigPatch {
    pub route_strategy: ConfigPatchValue<RouteStrategy>,
    pub default_request_timeout_ms: ConfigPatchValue<i64>,
    pub max_request_body_bytes: ConfigPatchValue<i64>,
    pub request_log_retention_days: ConfigPatchValue<i64>,
    pub daily_usage_retention_days: ConfigPatchValue<i64>,
    pub expose_debug_headers: ConfigPatchValue<bool>,
}

#[derive(Clone, Debug, Default)]
pub enum ConfigPatchValue<T> {
    #[default]
    Missing,
    Clear,
    Set(T),
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolvedRuntimeConfig {
    pub effective: RuntimeConfig,
    pub database: SystemConfig,
    pub fields: Vec<RuntimeConfigField>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub value: serde_json::Value,
    pub source: &'static str,
    pub database_value: Option<serde_json::Value>,
    pub environment_value: Option<serde_json::Value>,
    pub default_value: serde_json::Value,
    pub editable: bool,
    pub live_reload: bool,
    pub requires_restart: bool,
}

pub async fn get_system_config(pool: &SqlitePool) -> sqlx::Result<SystemConfig> {
    sqlx::query_as(
        "SELECT route_strategy, default_request_timeout_ms, max_request_body_bytes,
                request_log_retention_days, daily_usage_retention_days, expose_debug_headers,
                created_at, updated_at
         FROM system_config
         WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn upsert_system_config(
    pool: &SqlitePool,
    patch: &SystemConfigPatch,
) -> sqlx::Result<SystemConfig> {
    let mut conn = pool.acquire().await?;
    upsert_system_config_conn(&mut conn, patch).await
}

pub async fn upsert_system_config_conn(
    conn: &mut sqlx::SqliteConnection,
    patch: &SystemConfigPatch,
) -> sqlx::Result<SystemConfig> {
    let existing = get_system_config_conn(conn).await?;
    let now = now_string();
    let route_strategy = apply_config_patch(
        &patch.route_strategy,
        existing
            .route_strategy
            .as_deref()
            .and_then(|value| RouteStrategy::parse(value).ok()),
    )
    .map(|value| value.as_str().to_string());
    let default_request_timeout_ms = apply_config_patch(
        &patch.default_request_timeout_ms,
        existing.default_request_timeout_ms,
    );
    let max_request_body_bytes = apply_config_patch(
        &patch.max_request_body_bytes,
        existing.max_request_body_bytes,
    );
    let request_log_retention_days = apply_config_patch(
        &patch.request_log_retention_days,
        existing.request_log_retention_days,
    );
    let daily_usage_retention_days = apply_config_patch(
        &patch.daily_usage_retention_days,
        existing.daily_usage_retention_days,
    );
    let expose_debug_headers = apply_config_patch(
        &patch.expose_debug_headers,
        existing.expose_debug_headers.map(|v| v != 0),
    )
    .map(i64::from);

    sqlx::query(
        "UPDATE system_config
         SET route_strategy = ?,
             default_request_timeout_ms = ?,
             max_request_body_bytes = ?,
             request_log_retention_days = ?,
             daily_usage_retention_days = ?,
             expose_debug_headers = ?,
             updated_at = ?
         WHERE id = 1",
    )
    .bind(route_strategy)
    .bind(default_request_timeout_ms)
    .bind(max_request_body_bytes)
    .bind(request_log_retention_days)
    .bind(daily_usage_retention_days)
    .bind(expose_debug_headers)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    get_system_config_conn(conn).await
}

async fn get_system_config_conn(conn: &mut sqlx::SqliteConnection) -> sqlx::Result<SystemConfig> {
    sqlx::query_as(
        "SELECT route_strategy, default_request_timeout_ms, max_request_body_bytes,
                request_log_retention_days, daily_usage_retention_days, expose_debug_headers,
                created_at, updated_at
         FROM system_config
         WHERE id = 1",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn runtime_config(
    pool: &SqlitePool,
    config: &Config,
) -> sqlx::Result<ResolvedRuntimeConfig> {
    let database = get_system_config(pool).await?;
    Ok(resolve_runtime_config(config, database))
}

pub fn resolve_runtime_config(config: &Config, database: SystemConfig) -> ResolvedRuntimeConfig {
    let route_db = database
        .route_strategy
        .as_deref()
        .and_then(|value| RouteStrategy::parse(value).ok());
    let route = resolve_field(
        config.runtime_env.route_strategy,
        route_db,
        DEFAULT_ROUTE_STRATEGY,
    );
    let default_timeout = resolve_field(
        config.runtime_env.default_request_timeout_ms,
        database.default_request_timeout_ms,
        DEFAULT_REQUEST_TIMEOUT_MS,
    );
    let max_body = resolve_field(
        config.runtime_env.max_request_body_bytes,
        database.max_request_body_bytes,
        DEFAULT_MAX_REQUEST_BODY_BYTES,
    );
    let request_retention = resolve_field(
        config.runtime_env.request_log_retention_days,
        database.request_log_retention_days,
        DEFAULT_REQUEST_LOG_RETENTION_DAYS,
    );
    let daily_retention = resolve_field(
        config.runtime_env.daily_usage_retention_days,
        database.daily_usage_retention_days,
        DEFAULT_DAILY_USAGE_RETENTION_DAYS,
    );
    let debug_headers = resolve_field(
        config.runtime_env.expose_debug_headers,
        database.expose_debug_headers.map(|value| value != 0),
        DEFAULT_EXPOSE_DEBUG_HEADERS,
    );

    let effective = RuntimeConfig {
        route_strategy: route.value,
        default_request_timeout_ms: default_timeout.value,
        max_request_body_bytes: max_body.value,
        request_log_retention_days: request_retention.value,
        daily_usage_retention_days: daily_retention.value,
        expose_debug_headers: debug_headers.value,
    };
    let fields = vec![
        runtime_field(
            "route_strategy",
            "Default route strategy",
            serde_json::json!(route.value.as_str()),
            route.source,
            route_db.map(|value| serde_json::json!(value.as_str())),
            config
                .runtime_env
                .route_strategy
                .map(|value| serde_json::json!(value.as_str())),
            serde_json::json!(DEFAULT_ROUTE_STRATEGY.as_str()),
        ),
        runtime_field(
            "default_request_timeout_ms",
            "Default request timeout",
            serde_json::json!(default_timeout.value),
            default_timeout.source,
            database
                .default_request_timeout_ms
                .map(serde_json::Value::from),
            config
                .runtime_env
                .default_request_timeout_ms
                .map(serde_json::Value::from),
            serde_json::json!(DEFAULT_REQUEST_TIMEOUT_MS),
        ),
        runtime_field(
            "max_request_body_bytes",
            "Maximum request body size",
            serde_json::json!(max_body.value),
            max_body.source,
            database.max_request_body_bytes.map(serde_json::Value::from),
            config
                .runtime_env
                .max_request_body_bytes
                .map(serde_json::Value::from),
            serde_json::json!(DEFAULT_MAX_REQUEST_BODY_BYTES),
        ),
        runtime_field(
            "request_log_retention_days",
            "Request log retention",
            serde_json::json!(request_retention.value),
            request_retention.source,
            database
                .request_log_retention_days
                .map(serde_json::Value::from),
            config
                .runtime_env
                .request_log_retention_days
                .map(serde_json::Value::from),
            serde_json::json!(DEFAULT_REQUEST_LOG_RETENTION_DAYS),
        ),
        runtime_field(
            "daily_usage_retention_days",
            "Daily usage retention",
            serde_json::json!(daily_retention.value),
            daily_retention.source,
            database
                .daily_usage_retention_days
                .map(serde_json::Value::from),
            config
                .runtime_env
                .daily_usage_retention_days
                .map(serde_json::Value::from),
            serde_json::json!(DEFAULT_DAILY_USAGE_RETENTION_DAYS),
        ),
        runtime_field(
            "expose_debug_headers",
            "Expose debug headers",
            serde_json::json!(debug_headers.value),
            debug_headers.source,
            database
                .expose_debug_headers
                .map(|value| serde_json::json!(value != 0)),
            config
                .runtime_env
                .expose_debug_headers
                .map(serde_json::Value::from),
            serde_json::json!(DEFAULT_EXPOSE_DEBUG_HEADERS),
        ),
    ];

    ResolvedRuntimeConfig {
        effective,
        database,
        fields,
    }
}

#[derive(Clone, Copy)]
struct ResolvedField<T> {
    value: T,
    source: &'static str,
}

fn resolve_field<T: Copy>(env: Option<T>, database: Option<T>, default: T) -> ResolvedField<T> {
    if let Some(value) = env {
        return ResolvedField {
            value,
            source: "environment",
        };
    }
    if let Some(value) = database {
        return ResolvedField {
            value,
            source: "database",
        };
    }
    ResolvedField {
        value: default,
        source: "default",
    }
}

fn runtime_field(
    key: &'static str,
    label: &'static str,
    value: serde_json::Value,
    source: &'static str,
    database_value: Option<serde_json::Value>,
    environment_value: Option<serde_json::Value>,
    default_value: serde_json::Value,
) -> RuntimeConfigField {
    RuntimeConfigField {
        key,
        label,
        value,
        source,
        database_value,
        environment_value,
        default_value,
        editable: true,
        live_reload: true,
        requires_restart: false,
    }
}

fn apply_config_patch<T: Copy>(patch: &ConfigPatchValue<T>, current: Option<T>) -> Option<T> {
    match patch {
        ConfigPatchValue::Missing => current,
        ConfigPatchValue::Clear => None,
        ConfigPatchValue::Set(value) => Some(*value),
    }
}
