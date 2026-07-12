use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, admin_audit},
    contracts::{LimitPolicyResponse, RetentionResponse, RuntimeConfigFieldResponse},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/retention/run", post(admin_run_retention))
        .route(
            "/api/admin/settings",
            get(admin_settings).patch(admin_update_settings),
        )
}

async fn admin_run_retention(
    State(state): State<AppState>,
    Administrator(admin): Administrator,
) -> Result<Json<RetentionResponse>, ApiError> {
    let runtime = storage::runtime_config(&state.db, &state.config).await?;
    let request_log_retention_days = runtime.effective.request_log_retention_days;
    let daily_usage_retention_days = runtime.effective.daily_usage_retention_days;
    let result = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let result = storage::apply_retention_conn(
                conn,
                &storage::RetentionPolicy {
                    request_log_retention_days,
                    daily_usage_retention_days,
                },
                chrono::Utc::now(),
            )
            .await?;
            let audit = admin_audit(
                &admin,
                "run_retention",
                "retention",
                None,
                json!({
                    "request_log_retention_days": request_log_retention_days,
                    "daily_usage_retention_days": daily_usage_retention_days,
                    "request_logs_deleted": result.request_logs_deleted,
                    "daily_usage_deleted": result.daily_usage_deleted
                }),
            );
            Ok((result, audit))
        })
    })
    .await?;
    Ok(Json(result.into()))
}

#[derive(Serialize)]
struct SettingsSummary {
    service: &'static str,
    public_url: String,
    bind: String,
    log_level: String,
    route_strategy: String,
    default_request_timeout_ms: i64,
    max_request_body_bytes: i64,
    health_checks_enabled: bool,
    health_check_interval_ms: u64,
    request_log_retention_days: i64,
    daily_usage_retention_days: i64,
    retention_run_on_startup: bool,
    expose_debug_headers: bool,
    admin_email_configured: bool,
    bootstrap_admin_key_configured: bool,
    database: SettingsDatabase,
    counts: SettingsCounts,
    runtime: SettingsRuntime,
    environment: Vec<SettingsEnvironmentValue>,
    default_limit_policy: LimitPolicyResponse,
}

#[derive(Serialize)]
struct SettingsDatabase {
    kind: &'static str,
    configured: bool,
    settings: SettingsDatabaseValues,
}

#[derive(Serialize)]
struct SettingsDatabaseValues {
    route_strategy: Option<String>,
    default_request_timeout_ms: Option<i64>,
    max_request_body_bytes: Option<i64>,
    request_log_retention_days: Option<i64>,
    daily_usage_retention_days: Option<i64>,
    expose_debug_headers: Option<bool>,
    updated_at: String,
}

#[derive(Serialize)]
struct SettingsRuntime {
    precedence: &'static str,
    fields: Vec<RuntimeConfigFieldResponse>,
}

#[derive(Serialize)]
struct SettingsEnvironmentValue {
    key: &'static str,
    label: &'static str,
    value: Value,
    source: &'static str,
    editable: bool,
    requires_restart: bool,
}

#[derive(Serialize)]
struct SettingsCounts {
    users: i64,
    api_keys: i64,
    upstreams: i64,
    models: i64,
    request_logs: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
struct SettingsPatchRequest(Value);

impl SettingsPatchRequest {
    fn try_into_storage(self) -> Result<(storage::SystemConfigPatch, Vec<String>), ApiError> {
        parse_settings_patch(self.0)
    }
}

async fn admin_settings(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<SettingsSummary>, ApiError> {
    settings_summary(&state).await.map(Json)
}

async fn settings_summary(state: &AppState) -> Result<SettingsSummary, ApiError> {
    let runtime = storage::runtime_config(&state.db, &state.config).await?;
    let limits = storage::admin_limit_state(&state.db).await?;
    let counts = SettingsCounts {
        users: count_table(&state.db, "users").await?,
        api_keys: count_table(&state.db, "api_keys").await?,
        upstreams: count_table(&state.db, "upstreams").await?,
        models: count_table(&state.db, "models").await?,
        request_logs: count_table(&state.db, "request_logs").await?,
    };
    Ok(SettingsSummary {
        service: "codex-gateway",
        public_url: state.config.public_url.clone(),
        bind: state.config.bind.clone(),
        log_level: state.config.log_level.clone(),
        route_strategy: runtime.effective.route_strategy.as_str().to_string(),
        default_request_timeout_ms: runtime.effective.default_request_timeout_ms,
        max_request_body_bytes: runtime.effective.max_request_body_bytes,
        health_checks_enabled: state.config.health_checks_enabled,
        health_check_interval_ms: state.config.health_check_interval_ms,
        request_log_retention_days: runtime.effective.request_log_retention_days,
        daily_usage_retention_days: runtime.effective.daily_usage_retention_days,
        retention_run_on_startup: state.config.retention_run_on_startup,
        expose_debug_headers: runtime.effective.expose_debug_headers,
        admin_email_configured: state.config.admin_email.is_some(),
        bootstrap_admin_key_configured: state.config.bootstrap_admin_key.is_some(),
        database: SettingsDatabase {
            kind: "sqlite",
            configured: state.config.database_url != "sqlite://data/codex-gateway.db",
            settings: SettingsDatabaseValues {
                route_strategy: runtime.database.route_strategy,
                default_request_timeout_ms: runtime.database.default_request_timeout_ms,
                max_request_body_bytes: runtime.database.max_request_body_bytes,
                request_log_retention_days: runtime.database.request_log_retention_days,
                daily_usage_retention_days: runtime.database.daily_usage_retention_days,
                expose_debug_headers: runtime
                    .database
                    .expose_debug_headers
                    .map(|value| value != 0),
                updated_at: runtime.database.updated_at,
            },
        },
        counts,
        runtime: SettingsRuntime {
            precedence: "environment > database > default",
            fields: runtime.fields.into_iter().map(Into::into).collect(),
        },
        environment: environment_settings(&state.config),
        default_limit_policy: limits.system.into(),
    })
}

async fn admin_update_settings(
    State(state): State<AppState>,
    Administrator(admin): Administrator,
    payload: Result<Json<SettingsPatchRequest>, JsonRejection>,
) -> Result<Json<SettingsSummary>, ApiError> {
    let Json(input) = payload.map_err(json_rejection_error)?;
    let (patch, changed_fields) = input.try_into_storage()?;
    if changed_fields.is_empty() {
        return Err(ApiError::bad_request(
            "no settings fields supplied",
            "invalid_request",
        ));
    }
    storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let stored = storage::upsert_system_config_conn(conn, &patch).await?;
            let audit = admin_audit(
                &admin,
                "update_system_settings",
                "system_config",
                None,
                json!({
                    "changed_fields": changed_fields,
                    "stored_sources": "database",
                    "effective_precedence": "environment > database > default",
                    "requires_restart": false,
                    "updated_at": stored.updated_at
                }),
            );
            Ok(((), audit))
        })
    })
    .await?;
    settings_summary(&state).await.map(Json)
}

fn json_rejection_error(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::gateway(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds configured maximum",
            "request_body_too_large",
        );
    }
    ApiError::gateway(
        rejection.status(),
        "request body must be JSON",
        "invalid_request",
    )
}

fn environment_settings(config: &crate::config::Config) -> Vec<SettingsEnvironmentValue> {
    vec![
        environment_value(
            "bind",
            "Bind address",
            json!(config.bind),
            "environment_or_default",
        ),
        environment_value(
            "public_url",
            "Public URL",
            json!(config.public_url),
            "environment_or_default",
        ),
        environment_value(
            "log_level",
            "Log level",
            json!(config.log_level),
            "environment_or_default",
        ),
        environment_value(
            "health_checks_enabled",
            "Health checks enabled",
            json!(config.health_checks_enabled),
            "environment_or_default",
        ),
        environment_value(
            "health_check_interval_ms",
            "Health check interval",
            json!(config.health_check_interval_ms),
            "environment_or_default",
        ),
        environment_value(
            "retention_run_on_startup",
            "Startup retention",
            json!(config.retention_run_on_startup),
            "environment_or_default",
        ),
        environment_value(
            "admin_email_configured",
            "Admin email",
            json!(config.admin_email.is_some()),
            "environment_or_default",
        ),
        environment_value(
            "bootstrap_admin_key_configured",
            "Bootstrap key",
            json!(config.bootstrap_admin_key.is_some()),
            "environment_or_default",
        ),
    ]
}

fn environment_value(
    key: &'static str,
    label: &'static str,
    value: Value,
    source: &'static str,
) -> SettingsEnvironmentValue {
    SettingsEnvironmentValue {
        key,
        label,
        value,
        source,
        editable: false,
        requires_restart: true,
    }
}

async fn count_table(db: &sqlx::SqlitePool, table: &'static str) -> Result<i64, ApiError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(sqlx::query_scalar(&sql).fetch_one(db).await?)
}

fn parse_settings_patch(
    input: Value,
) -> Result<(storage::SystemConfigPatch, Vec<String>), ApiError> {
    let object = input.as_object().ok_or_else(|| {
        ApiError::bad_request("settings update must be a JSON object", "invalid_request")
    })?;
    let mut patch = storage::SystemConfigPatch::default();
    let mut changed = Vec::new();
    for (key, value) in object {
        match key.as_str() {
            "route_strategy" => {
                patch.route_strategy = parse_route_strategy_patch(value)?;
                changed.push(key.clone());
            }
            "default_request_timeout_ms" => {
                patch.default_request_timeout_ms = parse_positive_i64_patch(key, value)?;
                changed.push(key.clone());
            }
            "max_request_body_bytes" => {
                patch.max_request_body_bytes = parse_positive_i64_patch(key, value)?;
                changed.push(key.clone());
            }
            "request_log_retention_days" => {
                patch.request_log_retention_days = parse_non_negative_i64_patch(key, value)?;
                changed.push(key.clone());
            }
            "daily_usage_retention_days" => {
                patch.daily_usage_retention_days = parse_non_negative_i64_patch(key, value)?;
                changed.push(key.clone());
            }
            "expose_debug_headers" => {
                patch.expose_debug_headers = parse_bool_patch(key, value)?;
                changed.push(key.clone());
            }
            _ => {
                return Err(ApiError::bad_request(
                    format!("{key} is not a writable safe setting"),
                    "invalid_setting",
                ));
            }
        }
    }
    Ok((patch, changed))
}

fn parse_route_strategy_patch(
    value: &Value,
) -> Result<storage::ConfigPatchValue<crate::config::RouteStrategy>, ApiError> {
    if value.is_null() {
        return Ok(storage::ConfigPatchValue::Clear);
    }
    let value = value.as_str().ok_or_else(|| {
        ApiError::bad_request("route_strategy must be a string or null", "invalid_request")
    })?;
    let strategy = crate::config::RouteStrategy::parse(value).map_err(|_| {
        ApiError::bad_request(
            "route_strategy must be priority, weighted, or sticky_by_key",
            "invalid_request",
        )
    })?;
    Ok(storage::ConfigPatchValue::Set(strategy))
}

fn parse_positive_i64_patch(
    key: &str,
    value: &Value,
) -> Result<storage::ConfigPatchValue<i64>, ApiError> {
    if value.is_null() {
        return Ok(storage::ConfigPatchValue::Clear);
    }
    let parsed = value.as_i64().ok_or_else(|| {
        ApiError::bad_request(
            format!("{key} must be an integer or null"),
            "invalid_request",
        )
    })?;
    if parsed < 1 {
        return Err(ApiError::bad_request(
            format!("{key} must be at least 1"),
            "invalid_request",
        ));
    }
    Ok(storage::ConfigPatchValue::Set(parsed))
}

fn parse_non_negative_i64_patch(
    key: &str,
    value: &Value,
) -> Result<storage::ConfigPatchValue<i64>, ApiError> {
    if value.is_null() {
        return Ok(storage::ConfigPatchValue::Clear);
    }
    let parsed = value.as_i64().ok_or_else(|| {
        ApiError::bad_request(
            format!("{key} must be an integer or null"),
            "invalid_request",
        )
    })?;
    if parsed < 0 {
        return Err(ApiError::bad_request(
            format!("{key} must be zero or greater"),
            "invalid_request",
        ));
    }
    Ok(storage::ConfigPatchValue::Set(parsed))
}

fn parse_bool_patch(key: &str, value: &Value) -> Result<storage::ConfigPatchValue<bool>, ApiError> {
    if value.is_null() {
        return Ok(storage::ConfigPatchValue::Clear);
    }
    let parsed = value.as_bool().ok_or_else(|| {
        ApiError::bad_request(
            format!("{key} must be a boolean or null"),
            "invalid_request",
        )
    })?;
    Ok(storage::ConfigPatchValue::Set(parsed))
}
