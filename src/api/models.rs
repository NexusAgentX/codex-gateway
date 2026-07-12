use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use serde_json::json;

use crate::{
    AppState,
    storage::{self, UpdateModel, UpdateModelMapping, UpsertModel, UpsertModelMapping},
};

use super::{
    ApiError,
    auth::{Administrator, AdministratorJson, admin_audit, authenticate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(my_models))
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
}

async fn my_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<storage::Model>>, ApiError> {
    authenticate(&state, &headers).await?;
    Ok(Json(storage::list_visible_models(&state.db).await?))
}

async fn admin_models(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<storage::Model>>, ApiError> {
    Ok(Json(storage::list_models(&state.db).await?))
}

async fn admin_create_model(
    State(state): State<AppState>,
    AdministratorJson(admin, input): AdministratorJson<UpsertModel>,
) -> Result<Json<storage::Model>, ApiError> {
    validate_upsert_model(&state, &input).await?;
    let model = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let model = storage::create_model_conn(conn, &input).await?;
            let audit = admin_audit(
                &admin,
                "create_model",
                "model",
                Some(model.id.clone()),
                json!({ "public_name": model.public_name }),
            );
            Ok((model, audit))
        })
    })
    .await?;
    Ok(Json(model))
}

async fn admin_update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<UpdateModel>,
) -> Result<Json<storage::Model>, ApiError> {
    validate_update_model(&input)?;
    let model = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let model = storage::update_model_conn(conn, &id, &input)
                .await?
                .ok_or_else(|| {
                    ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found")
                })?;
            let audit = admin_audit(
                &admin,
                "update_model",
                "model",
                Some(id),
                json!({
                    "description_changed": input.description.is_some(),
                    "enabled_changed": input.enabled.is_some(),
                    "visible_to_users_changed": input.visible_to_users.is_some()
                }),
            );
            Ok((model, audit))
        })
    })
    .await?;
    Ok(Json(model))
}

async fn admin_model_mappings(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<storage::UpstreamModel>>, ApiError> {
    storage::get_model(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found"))?;
    Ok(Json(
        storage::list_upstream_models_for_model(&state.db, &id).await?,
    ))
}

async fn admin_create_model_mapping(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<UpsertModelMapping>,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    validate_model_mapping(&state, &input).await?;
    storage::get_model(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::gateway(StatusCode::NOT_FOUND, "model not found", "not_found"))?;
    let mapping = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let mapping = storage::create_upstream_model_conn(conn, &id, &input).await?;
            let audit = admin_audit(
                &admin,
                "create_model_mapping",
                "model_mapping",
                Some(mapping.id.clone()),
                json!({ "model_id": id, "upstream_id": mapping.upstream_id }),
            );
            Ok((mapping, audit))
        })
    })
    .await?;
    Ok(Json(mapping))
}

async fn admin_update_model_mapping(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AdministratorJson(admin, input): AdministratorJson<UpdateModelMapping>,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    validate_update_model_mapping(&state, &input).await?;
    let mapping = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let mapping = storage::update_upstream_model_conn(conn, &id, &input)
                .await?
                .ok_or_else(|| {
                    ApiError::gateway(
                        StatusCode::NOT_FOUND,
                        "model mapping not found",
                        "not_found",
                    )
                })?;
            let audit = admin_audit(
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
            );
            Ok((mapping, audit))
        })
    })
    .await?;
    Ok(Json(mapping))
}

async fn admin_disable_model_mapping(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Administrator(admin): Administrator,
) -> Result<Json<storage::UpstreamModel>, ApiError> {
    let input = UpdateModelMapping {
        upstream_id: None,
        upstream_model_name: None,
        enabled: Some(false),
        priority: None,
        weight: None,
    };
    let mapping = storage::with_admin_audit::<_, ApiError, _>(&state.db, move |conn| {
        Box::pin(async move {
            let mapping = storage::update_upstream_model_conn(conn, &id, &input)
                .await?
                .ok_or_else(|| {
                    ApiError::gateway(
                        StatusCode::NOT_FOUND,
                        "model mapping not found",
                        "not_found",
                    )
                })?;
            let audit = admin_audit(
                &admin,
                "disable_model_mapping",
                "model_mapping",
                Some(id),
                json!({ "enabled": false }),
            );
            Ok((mapping, audit))
        })
    })
    .await?;
    Ok(Json(mapping))
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
    validate_route_numbers(input.priority, input.weight)?;
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
    validate_route_numbers(input.priority, input.weight)?;
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

fn validate_route_numbers(priority: Option<i64>, weight: Option<i64>) -> Result<(), ApiError> {
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
    Ok(())
}
