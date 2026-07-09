use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    AppState, auth,
    storage::{
        self, CreateApiKey, CreateUser, ResetPassword, UpdateModel, UpdateModelMapping,
        UpdateUpstream, UpdateUser, UpsertModel, UpsertModelMapping, UpsertUpstream,
    },
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/overview", get(overview))
        .route("/api/api-keys", get(my_api_keys).post(create_my_api_key))
        .route("/api/api-keys/{id}/disable", post(disable_my_api_key))
        .route("/api/api-keys/{id}/revoke", post(revoke_my_api_key))
        .route("/api/requests", get(my_requests))
        .route("/api/usage/daily", get(my_usage))
        .route("/api/admin/users", get(admin_users).post(admin_create_user))
        .route("/api/admin/users/{id}", patch(admin_update_user))
        .route("/api/admin/users/{id}/password", post(admin_reset_password))
        .route(
            "/api/admin/api-keys",
            get(admin_api_keys).post(admin_create_api_key),
        )
        .route(
            "/api/admin/api-keys/{id}/disable",
            post(admin_disable_api_key),
        )
        .route(
            "/api/admin/api-keys/{id}/revoke",
            post(admin_revoke_api_key),
        )
        .route(
            "/api/admin/upstreams",
            get(admin_upstreams).post(admin_create_upstream),
        )
        .route("/api/admin/upstreams/{id}", patch(admin_update_upstream))
        .route(
            "/api/admin/upstreams/{id}/disable",
            post(admin_disable_upstream),
        )
        .route(
            "/api/admin/upstreams/{id}/health",
            post(admin_check_upstream_health),
        )
        .route(
            "/api/admin/models",
            get(admin_models).post(admin_create_model),
        )
        .route("/api/admin/models/{id}", patch(admin_update_model))
        .route(
            "/api/admin/models/{id}/mappings",
            get(admin_model_mappings).post(admin_create_model_mapping),
        )
        .route(
            "/api/admin/model-mappings/{id}",
            patch(admin_update_model_mapping),
        )
        .route(
            "/api/admin/model-mappings/{id}/disable",
            post(admin_disable_model_mapping),
        )
        .route("/api/admin/requests", get(admin_requests))
        .route("/api/admin/usage/daily", get(admin_usage))
        .route("/api/admin/settings", get(admin_settings))
        .route("/responses", post(crate::proxy::proxy_responses))
        .route("/v1/responses", post(crate::proxy::proxy_responses))
        .route("/responses/compact", post(crate::proxy::proxy_responses))
        .route("/v1/responses/compact", post(crate::proxy::proxy_responses))
        .route("/v1/models", get(crate::proxy::models))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Result<Json<Health>, ApiError> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(Health {
        status: "ok",
        service: "codex-gateway",
    }))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user: LoginUser,
    token: String,
    token_type: &'static str,
}

#[derive(Serialize)]
struct LoginUser {
    id: String,
    email: String,
    role: String,
}

async fn login(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = storage::find_user_credentials_by_email(&state.db, &input.email)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::UNAUTHORIZED, "invalid login", "invalid_login")
        })?;
    if user.status != "active" || !auth::verify_password(&input.password, &user.password_hash) {
        return Err(ApiError::gateway(
            StatusCode::UNAUTHORIZED,
            "invalid login",
            "invalid_login",
        ));
    }

    storage::mark_user_login(&state.db, &user.id).await?;
    let token = auth::generate_panel_token(&state.config.app_secret, &user.id);

    Ok(Json(LoginResponse {
        user: LoginUser {
            id: user.id,
            email: user.email,
            role: user.role,
        },
        token,
        token_type: "panel",
    }))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<auth::AuthenticatedUser>, ApiError> {
    Ok(Json(authenticate(&state, &headers).await?))
}

async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let usage = storage::list_daily_usage(&state.db, Some(&user.user_id)).await?;
    let requests = storage::list_request_logs(&state.db, Some(&user.user_id)).await?;
    Ok(Json(json!({
        "user": user,
        "daily_usage": usage,
        "recent_requests": requests.into_iter().take(20).collect::<Vec<_>>()
    })))
}

async fn my_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::ApiKeySummary>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_api_keys_for_user(&state.db, &user.user_id).await?,
    ))
}

#[derive(Serialize)]
struct CreatedApiKey {
    key: storage::ApiKeySummary,
    plaintext: String,
}

async fn create_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKey>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    validate_create_api_key(&input)?;
    let (key, plaintext) =
        storage::create_api_key(&state.db, &state.config.app_secret, &user.user_id, &input).await?;
    Ok(Json(CreatedApiKey { key, plaintext }))
}

async fn disable_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    update_my_api_key_status(state, headers, id, "disabled").await
}

async fn revoke_my_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    update_my_api_key_status(state, headers, id, "revoked").await
}

async fn update_my_api_key_status(
    state: AppState,
    headers: HeaderMap,
    id: String,
    status: &'static str,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let key = storage::get_api_key(&state.db, &id).await?.ok_or_else(|| {
        ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
    })?;
    if key.user_id != user.user_id {
        return Err(ApiError::forbidden(
            "API key does not belong to user",
            "forbidden",
        ));
    }
    let updated = storage::set_api_key_status(&state.db, &id, status)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
        })?;
    Ok(Json(updated))
}

async fn my_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_request_logs(&state.db, Some(&user.user_id)).await?,
    ))
}

async fn my_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_daily_usage(&state.db, Some(&user.user_id)).await?,
    ))
}

async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::User>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_users(&state.db).await?))
}

async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateUser>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_create_user(&input)?;
    let id = storage::ensure_user(&state.db, &input).await?;
    audit_admin_mutation(
        &state,
        &admin,
        "create_user",
        "user",
        Some(id.clone()),
        json!({ "email": input.email, "role": input.role }),
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

async fn admin_update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateUser>,
) -> Result<Json<storage::User>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_update_user(&input)?;
    let user = storage::update_user(&state.db, &id, &input)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    audit_admin_mutation(
        &state,
        &admin,
        "update_user",
        "user",
        Some(id),
        json!({
            "role_changed": input.role.is_some(),
            "status_changed": input.status.is_some(),
            "display_name_changed": input.display_name.is_some()
        }),
    )
    .await?;
    Ok(Json(user))
}

async fn admin_reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ResetPassword>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_password(&input.password)?;
    if !storage::reset_user_password(&state.db, &id, &input.password).await? {
        return Err(ApiError::gateway(
            StatusCode::NOT_FOUND,
            "user not found",
            "not_found",
        ));
    }
    audit_admin_mutation(
        &state,
        &admin,
        "reset_user_password",
        "user",
        Some(id.clone()),
        json!({ "password_reset": true }),
    )
    .await?;
    Ok(Json(json!({ "id": id, "password_reset": true })))
}

async fn admin_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::ApiKeySummary>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_api_keys(&state.db).await?))
}

#[derive(Deserialize)]
struct AdminCreateApiKey {
    user_id: String,
    name: String,
    expires_at: Option<String>,
}

async fn admin_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AdminCreateApiKey>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    let create = CreateApiKey {
        name: input.name,
        expires_at: input.expires_at,
    };
    validate_create_api_key(&create)?;
    storage::get_user(&state.db, &input.user_id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "user not found", "not_found"))?;
    let (key, plaintext) =
        storage::create_api_key(&state.db, &state.config.app_secret, &input.user_id, &create)
            .await?;
    audit_admin_mutation(
        &state,
        &admin,
        "create_api_key",
        "api_key",
        Some(key.id.clone()),
        json!({ "user_id": input.user_id, "name": key.name, "expires_at_set": key.expires_at.is_some() }),
    )
    .await?;
    Ok(Json(CreatedApiKey { key, plaintext }))
}

async fn admin_disable_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    update_admin_api_key_status(state, headers, id, "disabled").await
}

async fn admin_revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    update_admin_api_key_status(state, headers, id, "revoked").await
}

async fn update_admin_api_key_status(
    state: AppState,
    headers: HeaderMap,
    id: String,
    status: &'static str,
) -> Result<Json<storage::ApiKeySummary>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    let updated = storage::set_api_key_status(&state.db, &id, status)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "API key not found", "not_found")
        })?;
    audit_admin_mutation(
        &state,
        &admin,
        if status == "revoked" {
            "revoke_api_key"
        } else {
            "disable_api_key"
        },
        "api_key",
        Some(id),
        json!({ "status": status }),
    )
    .await?;
    Ok(Json(updated))
}

async fn admin_upstreams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::Upstream>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_upstreams(&state.db).await?))
}

async fn admin_create_upstream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpsertUpstream>,
) -> Result<Json<storage::Upstream>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_upsert_upstream(&input)?;
    let upstream = storage::create_upstream(
        &state.db,
        &state.config.app_secret,
        state.config.secret_key_version,
        &input,
    )
    .await?;
    audit_admin_mutation(
        &state,
        &admin,
        "create_upstream",
        "upstream",
        Some(upstream.id.clone()),
        json!({
            "name": upstream.name,
            "base_url": upstream.base_url,
            "secret_version": upstream.api_key_secret_version
        }),
    )
    .await?;
    Ok(Json(upstream))
}

async fn admin_update_upstream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateUpstream>,
) -> Result<Json<storage::Upstream>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_update_upstream(&input)?;
    let upstream = storage::update_upstream(
        &state.db,
        &state.config.app_secret,
        state.config.secret_key_version,
        &id,
        &input,
    )
    .await?
    .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found"))?;
    audit_admin_mutation(
        &state,
        &admin,
        "update_upstream",
        "upstream",
        Some(id),
        json!({
            "name_changed": input.name.is_some(),
            "base_url_changed": input.base_url.is_some(),
            "api_key_rotated": input.api_key.is_some(),
            "enabled_changed": input.enabled.is_some(),
            "priority_changed": input.priority.is_some(),
            "weight_changed": input.weight.is_some(),
            "timeout_ms_changed": input.timeout_ms.is_some(),
            "max_retries_changed": input.max_retries.is_some(),
            "health_check_path_changed": input.health_check_path.is_some(),
            "secret_version": upstream.api_key_secret_version
        }),
    )
    .await?;
    Ok(Json(upstream))
}

async fn admin_disable_upstream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::Upstream>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    let input = UpdateUpstream {
        name: None,
        base_url: None,
        api_key: None,
        enabled: Some(false),
        priority: None,
        weight: None,
        timeout_ms: None,
        max_retries: None,
        health_check_path: None,
    };
    let upstream = storage::update_upstream(
        &state.db,
        &state.config.app_secret,
        state.config.secret_key_version,
        &id,
        &input,
    )
    .await?
    .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found"))?;
    audit_admin_mutation(
        &state,
        &admin,
        "disable_upstream",
        "upstream",
        Some(id),
        json!({ "enabled": false }),
    )
    .await?;
    Ok(Json(upstream))
}

async fn admin_check_upstream_health(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    let upstream = storage::get_upstream(&state.db, &id)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(StatusCode::NOT_FOUND, "upstream not found", "not_found")
        })?;
    let status = crate::upstream::check_upstream_health(
        &state.http,
        &state.db,
        &state.config.app_secret,
        &upstream,
    )
    .await
    .map_err(|error| {
        tracing::warn!(?error, upstream_id = %id, "upstream health check failed");
        ApiError::gateway(
            StatusCode::BAD_GATEWAY,
            "upstream health check failed",
            "upstream_unavailable",
        )
    })?;
    audit_admin_mutation(
        &state,
        &admin,
        "check_upstream_health",
        "upstream",
        Some(id.clone()),
        json!({ "health": status }),
    )
    .await?;
    Ok(Json(json!({ "id": id, "health": status })))
}

async fn admin_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::Model>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_models(&state.db).await?))
}

async fn admin_create_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpsertModel>,
) -> Result<Json<storage::Model>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_upsert_model(&state, &input).await?;
    let model = storage::create_model(&state.db, &input).await?;
    audit_admin_mutation(
        &state,
        &admin,
        "create_model",
        "model",
        Some(model.id.clone()),
        json!({ "public_name": model.public_name }),
    )
    .await?;
    Ok(Json(model))
}

async fn admin_update_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateModel>,
) -> Result<Json<storage::Model>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_update_model(&input)?;
    let model = storage::update_model(&state.db, &id, &input)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found"))?;
    audit_admin_mutation(
        &state,
        &admin,
        "update_model",
        "model",
        Some(id),
        json!({
            "description_changed": input.description.is_some(),
            "enabled_changed": input.enabled.is_some(),
            "visible_to_users_changed": input.visible_to_users.is_some()
        }),
    )
    .await?;
    Ok(Json(model))
}

async fn admin_model_mappings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<storage::UpstreamModel>>, ApiError> {
    require_admin(&state, &headers).await?;
    storage::get_model(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found"))?;
    Ok(Json(
        storage::list_upstream_models_for_model(&state.db, &id).await?,
    ))
}

async fn admin_create_model_mapping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpsertModelMapping>,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_model_mapping(&state, &input).await?;
    storage::get_model(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found"))?;
    let mapping = storage::create_upstream_model(&state.db, &id, &input).await?;
    audit_admin_mutation(
        &state,
        &admin,
        "create_model_mapping",
        "model_mapping",
        Some(mapping.id.clone()),
        json!({ "model_id": id, "upstream_id": mapping.upstream_id }),
    )
    .await?;
    Ok(Json(mapping))
}

async fn admin_update_model_mapping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateModelMapping>,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    validate_update_model_mapping(&state, &input).await?;
    let mapping = storage::update_upstream_model(&state.db, &id, &input)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(
                StatusCode::NOT_FOUND,
                "model mapping not found",
                "not_found",
            )
        })?;
    audit_admin_mutation(
        &state,
        &admin,
        "update_model_mapping",
        "model_mapping",
        Some(id),
        json!({
            "upstream_id_changed": input.upstream_id.is_some(),
            "upstream_model_name_changed": input.upstream_model_name.is_some(),
            "enabled_changed": input.enabled.is_some(),
            "priority_changed": input.priority.is_some(),
            "weight_changed": input.weight.is_some()
        }),
    )
    .await?;
    Ok(Json(mapping))
}

async fn admin_disable_model_mapping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    let input = UpdateModelMapping {
        upstream_id: None,
        upstream_model_name: None,
        enabled: Some(false),
        priority: None,
        weight: None,
    };
    let mapping = storage::update_upstream_model(&state.db, &id, &input)
        .await?
        .ok_or_else(|| {
            ApiError::gateway(
                StatusCode::NOT_FOUND,
                "model mapping not found",
                "not_found",
            )
        })?;
    audit_admin_mutation(
        &state,
        &admin,
        "disable_model_mapping",
        "model_mapping",
        Some(id),
        json!({ "enabled": false }),
    )
    .await?;
    Ok(Json(mapping))
}

async fn admin_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_request_logs(&state.db, None).await?))
}

async fn admin_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    require_admin(&state, &headers).await?;
    Ok(Json(storage::list_daily_usage(&state.db, None).await?))
}

#[derive(Serialize)]
struct SettingsSummary {
    service: &'static str,
    public_url: String,
    bind: String,
    log_level: String,
    route_strategy: &'static str,
    health_checks_enabled: bool,
    health_check_interval_ms: u64,
    admin_email_configured: bool,
    bootstrap_admin_key_configured: bool,
    database: SettingsDatabase,
    counts: SettingsCounts,
}

#[derive(Serialize)]
struct SettingsDatabase {
    kind: &'static str,
    configured: bool,
}

#[derive(Serialize)]
struct SettingsCounts {
    users: i64,
    api_keys: i64,
    upstreams: i64,
    models: i64,
    request_logs: i64,
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SettingsSummary>, ApiError> {
    require_admin(&state, &headers).await?;
    let counts = SettingsCounts {
        users: count_table(&state.db, "users").await?,
        api_keys: count_table(&state.db, "api_keys").await?,
        upstreams: count_table(&state.db, "upstreams").await?,
        models: count_table(&state.db, "models").await?,
        request_logs: count_table(&state.db, "request_logs").await?,
    };
    Ok(Json(SettingsSummary {
        service: "codex-gateway",
        public_url: state.config.public_url.clone(),
        bind: state.config.bind.clone(),
        log_level: state.config.log_level.clone(),
        route_strategy: match state.config.route_strategy {
            crate::config::RouteStrategy::Priority => "priority",
            crate::config::RouteStrategy::Weighted => "weighted",
            crate::config::RouteStrategy::StickyByKey => "sticky_by_key",
        },
        health_checks_enabled: state.config.health_checks_enabled,
        health_check_interval_ms: state.config.health_check_interval_ms,
        admin_email_configured: state.config.admin_email.is_some(),
        bootstrap_admin_key_configured: state.config.bootstrap_admin_key.is_some(),
        database: SettingsDatabase {
            kind: "sqlite",
            configured: state.config.database_url != "sqlite://data/codex-gateway.db",
        },
        counts,
    }))
}

async fn count_table(db: &sqlx::SqlitePool, table: &'static str) -> Result<i64, ApiError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(sqlx::query_scalar(&sql).fetch_one(db).await?)
}

async fn audit_admin_mutation(
    state: &AppState,
    actor: &auth::AuthenticatedUser,
    action: &'static str,
    resource_type: &'static str,
    resource_id: Option<String>,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    storage::insert_admin_audit_log(
        &state.db,
        storage::AdminAuditInsert {
            actor_user_id: actor.user_id.clone(),
            actor_email: actor.email.clone(),
            action,
            resource_type,
            resource_id,
            status: "success",
            metadata_json: Some(metadata.to_string()),
        },
    )
    .await?;
    Ok(())
}

pub async fn authenticate_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    auth::authenticate_api_key(&state.db, &state.config.app_secret, header)
        .await
        .map_err(ApiError::from_auth)
}

pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let plaintext = auth::parse_bearer(header).map_err(ApiError::from_auth)?;
    if !auth::is_panel_token(plaintext) {
        return auth::authenticate_api_key(&state.db, &state.config.app_secret, header)
            .await
            .map_err(ApiError::from_auth);
    }

    let (user_id, session_id) = auth::verify_panel_token(&state.config.app_secret, plaintext)
        .map_err(ApiError::from_auth)?;
    let user = storage::get_user(&state.db, &user_id)
        .await?
        .ok_or_else(|| ApiError::from_auth(auth::AuthError::Invalid))?;
    if user.status != "active" {
        return Err(ApiError::from_auth(auth::AuthError::DisabledUser));
    }
    Ok(auth::AuthenticatedUser {
        user_id: user.id,
        api_key_id: format!("panel:{session_id}"),
        key_prefix: "panel".to_string(),
        email: user.email,
        role: user.role,
    })
}

pub async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let user = authenticate(state, headers).await?;
    if auth::is_admin(&user) {
        Ok(user)
    } else {
        Err(ApiError::forbidden("admin role required", "forbidden"))
    }
}

fn validate_create_user(input: &CreateUser) -> Result<(), ApiError> {
    validate_email(&input.email)?;
    validate_password(&input.password)?;
    validate_role(&input.role)?;
    validate_optional_name("display_name", input.display_name.as_deref())?;
    Ok(())
}

fn validate_update_user(input: &UpdateUser) -> Result<(), ApiError> {
    if input.role.is_none() && input.status.is_none() && input.display_name.is_none() {
        return Err(ApiError::bad_request(
            "no user fields supplied",
            "invalid_request",
        ));
    }
    if let Some(role) = &input.role {
        validate_role(role)?;
    }
    if let Some(status) = &input.status {
        validate_user_status(status)?;
    }
    validate_optional_name("display_name", input.display_name.as_deref())?;
    Ok(())
}

fn validate_create_api_key(input: &CreateApiKey) -> Result<(), ApiError> {
    validate_required("name", &input.name)?;
    if let Some(expires_at) = &input.expires_at {
        chrono::DateTime::parse_from_rfc3339(expires_at).map_err(|_| {
            ApiError::bad_request("expires_at must be an RFC3339 timestamp", "invalid_request")
        })?;
    }
    Ok(())
}

fn validate_upsert_upstream(input: &UpsertUpstream) -> Result<(), ApiError> {
    validate_required("name", &input.name)?;
    validate_url(&input.base_url)?;
    validate_upstream_api_key(&input.api_key)?;
    validate_route_numbers(
        input.priority,
        input.weight,
        input.timeout_ms,
        input.max_retries,
    )?;
    validate_health_path(input.health_check_path.as_deref())?;
    Ok(())
}

fn validate_update_upstream(input: &UpdateUpstream) -> Result<(), ApiError> {
    if input.name.is_none()
        && input.base_url.is_none()
        && input.api_key.is_none()
        && input.enabled.is_none()
        && input.priority.is_none()
        && input.weight.is_none()
        && input.timeout_ms.is_none()
        && input.max_retries.is_none()
        && input.health_check_path.is_none()
    {
        return Err(ApiError::bad_request(
            "no upstream fields supplied",
            "invalid_request",
        ));
    }
    if let Some(name) = &input.name {
        validate_required("name", name)?;
    }
    if let Some(base_url) = &input.base_url {
        validate_url(base_url)?;
    }
    if let Some(api_key) = &input.api_key {
        validate_upstream_api_key(api_key)?;
    }
    validate_route_numbers(
        input.priority,
        input.weight,
        input.timeout_ms,
        input.max_retries,
    )?;
    validate_health_path(input.health_check_path.as_deref())?;
    Ok(())
}

async fn validate_upsert_model(state: &AppState, input: &UpsertModel) -> Result<(), ApiError> {
    validate_required("public_name", &input.public_name)?;
    if let Some(mappings) = &input.upstream_mappings {
        for mapping in mappings {
            validate_model_mapping(state, mapping).await?;
        }
    }
    Ok(())
}

fn validate_update_model(input: &UpdateModel) -> Result<(), ApiError> {
    if input.description.is_none() && input.enabled.is_none() && input.visible_to_users.is_none() {
        return Err(ApiError::bad_request(
            "no model fields supplied",
            "invalid_request",
        ));
    }
    Ok(())
}

async fn validate_model_mapping(
    state: &AppState,
    input: &UpsertModelMapping,
) -> Result<(), ApiError> {
    validate_required("upstream_id", &input.upstream_id)?;
    storage::get_upstream(&state.db, &input.upstream_id)
        .await?
        .ok_or_else(|| ApiError::bad_request("upstream_id does not exist", "invalid_request"))?;
    validate_required("upstream_model_name", &input.upstream_model_name)?;
    validate_route_numbers(input.priority, input.weight, None, None)?;
    Ok(())
}

async fn validate_update_model_mapping(
    state: &AppState,
    input: &UpdateModelMapping,
) -> Result<(), ApiError> {
    if input.upstream_id.is_none()
        && input.upstream_model_name.is_none()
        && input.enabled.is_none()
        && input.priority.is_none()
        && input.weight.is_none()
    {
        return Err(ApiError::bad_request(
            "no model mapping fields supplied",
            "invalid_request",
        ));
    }
    if let Some(upstream_id) = &input.upstream_id {
        validate_required("upstream_id", upstream_id)?;
        storage::get_upstream(&state.db, upstream_id)
            .await?
            .ok_or_else(|| {
                ApiError::bad_request("upstream_id does not exist", "invalid_request")
            })?;
    }
    if let Some(name) = &input.upstream_model_name {
        validate_required("upstream_model_name", name)?;
    }
    validate_route_numbers(input.priority, input.weight, None, None)?;
    Ok(())
}

fn validate_email(email: &str) -> Result<(), ApiError> {
    validate_required("email", email)?;
    if !email.contains('@') || email.contains(char::is_whitespace) {
        return Err(ApiError::bad_request(
            "email must be a valid address",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> Result<(), ApiError> {
    if !matches!(role, "admin" | "user") {
        return Err(ApiError::bad_request(
            "role must be admin or user",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_user_status(status: &str) -> Result<(), ApiError> {
    if !matches!(status, "active" | "disabled") {
        return Err(ApiError::bad_request(
            "status must be active or disabled",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_optional_name(field: &str, value: Option<&str>) -> Result<(), ApiError> {
    if let Some(value) = value {
        validate_required(field, value)?;
    }
    Ok(())
}

fn validate_required(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(
            format!("{field} must not be empty"),
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_url(value: &str) -> Result<(), ApiError> {
    validate_required("base_url", value)?;
    let parsed = url::Url::parse(value)
        .map_err(|_| ApiError::bad_request("base_url must be a valid URL", "invalid_request"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "base_url must use http or https",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_upstream_api_key(value: &str) -> Result<(), ApiError> {
    validate_required("api_key", value)?;
    crate::proxy::upstream_authorization_header(value).map_err(|_| {
        ApiError::bad_request(
            "api_key cannot be used in an Authorization header",
            "invalid_request",
        )
    })?;
    Ok(())
}

fn validate_route_numbers(
    priority: Option<i64>,
    weight: Option<i64>,
    timeout_ms: Option<i64>,
    max_retries: Option<i64>,
) -> Result<(), ApiError> {
    if priority.is_some_and(|value| value < 0) {
        return Err(ApiError::bad_request(
            "priority must be zero or greater",
            "invalid_request",
        ));
    }
    if weight.is_some_and(|value| value < 1) {
        return Err(ApiError::bad_request(
            "weight must be at least 1",
            "invalid_request",
        ));
    }
    if timeout_ms.is_some_and(|value| value < 1) {
        return Err(ApiError::bad_request(
            "timeout_ms must be at least 1",
            "invalid_request",
        ));
    }
    if max_retries.is_some_and(|value| value < 0) {
        return Err(ApiError::bad_request(
            "max_retries must be zero or greater",
            "invalid_request",
        ));
    }
    Ok(())
}

fn validate_health_path(value: Option<&str>) -> Result<(), ApiError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_required("health_check_path", value)?;
    if !value.starts_with('/') {
        return Err(ApiError::bad_request(
            "health_check_path must start with /",
            "invalid_request",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    kind: &'static str,
    code: &'static str,
}

impl ApiError {
    pub fn gateway(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            kind: "gateway_error",
            code,
        }
    }

    pub fn bad_request(message: impl Into<String>, code: &'static str) -> Self {
        Self::gateway(StatusCode::BAD_REQUEST, message, code)
    }

    pub fn forbidden(message: impl Into<String>, code: &'static str) -> Self {
        Self::gateway(StatusCode::FORBIDDEN, message, code)
    }

    pub fn from_auth(error: auth::AuthError) -> Self {
        match error {
            auth::AuthError::Missing | auth::AuthError::Invalid => Self::gateway(
                StatusCode::UNAUTHORIZED,
                "invalid API key",
                "invalid_api_key",
            ),
            auth::AuthError::Disabled => Self::gateway(
                StatusCode::FORBIDDEN,
                "disabled API key",
                "disabled_api_key",
            ),
            auth::AuthError::Expired => {
                Self::gateway(StatusCode::FORBIDDEN, "expired API key", "expired_api_key")
            }
            auth::AuthError::DisabledUser => {
                Self::gateway(StatusCode::FORBIDDEN, "disabled user", "disabled_user")
            }
            auth::AuthError::Storage(_) => Self::gateway(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway storage error",
                "gateway_internal_error",
            ),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(?error, "database error");
        Self::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gateway storage error",
            "gateway_internal_error",
        )
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(?error, "gateway error");
        Self::gateway(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gateway internal error",
            "gateway_internal_error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "type": self.kind,
                "code": self.code
            }
        }));
        (self.status, body).into_response()
    }
}
