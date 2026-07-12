use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AppState, storage};

use super::{
    ApiError,
    auth::{Administrator, authenticate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/overview", get(overview))
        .route("/api/requests", get(my_requests))
        .route("/api/analytics", get(my_analytics))
        .route("/api/usage/daily", get(my_usage))
        .route("/api/usage/summary", get(my_usage_summary))
        .route("/api/admin/requests", get(admin_requests))
        .route("/api/admin/analytics", get(admin_analytics))
        .route("/api/admin/usage/daily", get(admin_usage))
        .route("/api/admin/usage/summary", get(admin_usage_summary))
        .route("/api/admin/metrics", get(admin_metrics))
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

async fn my_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RequestLogQuery>,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let filters = request_log_filters(query, Some(user.user_id))?;
    Ok(Json(
        storage::list_request_logs_filtered(&state.db, &filters).await?,
    ))
}

async fn my_analytics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RequestLogQuery>,
) -> Result<Json<storage::AnalyticsSnapshot>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let filters = request_log_filters(query, Some(user.user_id))?;
    let mut analytics = storage::analytics_snapshot(&state.db, &filters).await?;
    analytics.user_error_rate.clear();
    Ok(Json(analytics))
}

async fn my_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::list_daily_usage_filtered(
            &state.db,
            &daily_usage_filters(query, Some(user.user_id))?,
        )
        .await?,
    ))
}

async fn my_usage_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<storage::UsageSummary>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    Ok(Json(
        storage::usage_summary(&state.db, &daily_usage_filters(query, Some(user.user_id))?).await?,
    ))
}

async fn admin_requests(
    State(state): State<AppState>,
    Query(query): Query<RequestLogQuery>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<storage::RequestLogRow>>, ApiError> {
    let filters = request_log_filters(query, None)?;
    Ok(Json(
        storage::list_request_logs_filtered(&state.db, &filters).await?,
    ))
}

async fn admin_analytics(
    State(state): State<AppState>,
    Query(query): Query<RequestLogQuery>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::AnalyticsSnapshot>, ApiError> {
    let filters = request_log_filters(query, None)?;
    Ok(Json(
        storage::analytics_snapshot(&state.db, &filters).await?,
    ))
}

async fn admin_usage(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
    Administrator(_admin): Administrator,
) -> Result<Json<Vec<storage::DailyUsageRow>>, ApiError> {
    Ok(Json(
        storage::list_daily_usage_filtered(&state.db, &daily_usage_filters(query, None)?).await?,
    ))
}

async fn admin_usage_summary(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::UsageSummary>, ApiError> {
    Ok(Json(
        storage::usage_summary(&state.db, &daily_usage_filters(query, None)?).await?,
    ))
}

async fn admin_metrics(
    State(state): State<AppState>,
    Administrator(_admin): Administrator,
) -> Result<Json<storage::GatewayMetrics>, ApiError> {
    Ok(Json(storage::gateway_metrics(&state.db).await?))
}

#[derive(Default, Deserialize)]
struct UsageQuery {
    user_id: Option<String>,
    key_id: Option<String>,
    api_key_id: Option<String>,
    model_id: Option<String>,
    upstream_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<i64>,
}

fn daily_usage_filters(
    query: UsageQuery,
    scoped_user_id: Option<String>,
) -> Result<storage::DailyUsageFilters, ApiError> {
    let user_id = scoped_user_id.or(query.user_id);
    Ok(storage::DailyUsageFilters {
        user_id: clean_optional(user_id),
        api_key_id: clean_optional(query.api_key_id.or(query.key_id)),
        model_id: clean_optional(query.model_id),
        upstream_id: clean_optional(query.upstream_id),
        date_from: parse_usage_date_bound(query.from.as_deref())?,
        date_to: parse_usage_date_bound(query.to.as_deref())?,
        limit: query.limit.map(|value| value.clamp(1, 1000)),
    })
}

fn parse_usage_date_bound(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(
            timestamp
                .with_timezone(&chrono::Utc)
                .date_naive()
                .to_string(),
        ));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(Some(date.to_string()));
    }
    Err(ApiError::bad_request(
        "usage date filters must be RFC3339 timestamps or YYYY-MM-DD dates",
        "invalid_request",
    ))
}

#[derive(Default, Deserialize)]
struct RequestLogQuery {
    user_id: Option<String>,
    key_id: Option<String>,
    api_key_id: Option<String>,
    model_id: Option<String>,
    upstream_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
    latency_min_ms: Option<i64>,
    latency_max_ms: Option<i64>,
    limit: Option<i64>,
}

fn request_log_filters(
    query: RequestLogQuery,
    scoped_user_id: Option<String>,
) -> Result<storage::RequestLogFilters, ApiError> {
    let user_id = scoped_user_id.or(query.user_id);
    let (status_code, error_only) = parse_status_filter(query.status.as_deref())?;
    Ok(storage::RequestLogFilters {
        user_id: clean_optional(user_id),
        api_key_id: clean_optional(query.api_key_id.or(query.key_id)),
        model_id: clean_optional(query.model_id),
        upstream_id: clean_optional(query.upstream_id),
        status_code,
        error_only,
        started_at_from: parse_date_bound(query.from.as_deref(), false)?,
        started_at_to: parse_date_bound(query.to.as_deref(), true)?,
        latency_min_ms: query.latency_min_ms.map(|value| value.max(0)),
        latency_max_ms: query.latency_max_ms.map(|value| value.max(0)),
        limit: query.limit.map(|value| value.clamp(1, 1000)),
    })
}

fn parse_status_filter(value: Option<&str>) -> Result<(Option<i64>, bool), ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok((None, false));
    };
    if value.eq_ignore_ascii_case("error") || value.eq_ignore_ascii_case("errors") {
        return Ok((None, true));
    }
    let status = value.parse::<i64>().map_err(|_| {
        ApiError::bad_request("status must be an HTTP code or error", "invalid_request")
    })?;
    Ok((Some(status), false))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn parse_date_bound(value: Option<&str>, end_of_day: bool) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(
            timestamp
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let time = if end_of_day {
            chrono::NaiveTime::from_hms_milli_opt(23, 59, 59, 999).unwrap()
        } else {
            chrono::NaiveTime::MIN
        };
        return Ok(Some(
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                date.and_time(time),
                chrono::Utc,
            )
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ));
    }
    Err(ApiError::bad_request(
        "date filters must be RFC3339 timestamps or YYYY-MM-DD dates",
        "invalid_request",
    ))
}
