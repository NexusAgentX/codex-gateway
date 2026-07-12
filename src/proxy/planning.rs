use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::{
    AppState, auth,
    config::RouteStrategy,
    http_error::ApiError,
    routing::{self, RouteCandidate, RoutingError},
};

use super::request::PreparedRequest;

pub(super) struct RoutePlan {
    pub(super) candidates: Vec<RouteCandidate>,
    pub(super) route_strategy: String,
    pub(super) route_key_hash: String,
}

impl RoutePlan {
    pub(super) fn max_retries(&self) -> i64 {
        self.candidates
            .first()
            .map(|candidate| candidate.max_retries.max(0))
            .unwrap_or_default()
    }
}

pub(super) async fn plan(
    state: &AppState,
    request: &PreparedRequest,
) -> Result<RoutePlan, ApiError> {
    let candidates = routing::route_candidates(
        &state.db,
        &state.config,
        &request.model,
        request.runtime.default_request_timeout_ms,
    )
    .await
    .map_err(|error| route_error(&request.model, RoutingError::Storage(error)))?;
    if candidates.is_empty() {
        let model_exists = routing::model_exists(&state.db, &request.model)
            .await
            .map_err(|error| route_error(&request.model, RoutingError::Storage(error)))?;
        return Err(if model_exists {
            unavailable_error(&request.model)
        } else {
            route_error(&request.model, RoutingError::ModelNotFound)
        });
    }

    let route_seed = uuid::Uuid::new_v4().to_string();
    let route_key = routing_key(
        request.runtime.route_strategy,
        &request.model,
        &request.json,
        &request.user.api_key_id,
        &route_seed,
    );
    let route_key_hash = auth::hash_api_key(&state.config.app_secret, &route_key);
    let candidates =
        routing::order_candidates(&candidates, request.runtime.route_strategy, &route_key);
    Ok(RoutePlan {
        candidates,
        route_strategy: request.runtime.route_strategy.as_str().to_string(),
        route_key_hash,
    })
}

pub(super) fn route_decision_json(
    route: &RouteCandidate,
    index: usize,
    candidate_count: usize,
    retries_remaining: i64,
    route_key_hash: &str,
) -> String {
    json!({
        "attempt": index + 1,
        "candidate_count": candidate_count,
        "route_key_hash": route_key_hash,
        "model_id": route.model_id,
        "upstream_id": route.upstream_id,
        "upstream_model_id": route.upstream_model_id,
        "upstream_model_name": route.upstream_model_name,
        "upstream_priority": route.upstream_priority,
        "upstream_model_priority": route.upstream_model_priority,
        "upstream_weight": route.upstream_weight,
        "upstream_model_weight": route.upstream_model_weight,
        "max_retries": route.max_retries.max(0),
        "retries_remaining_after_this_attempt": retries_remaining
    })
    .to_string()
}

fn routing_key(
    strategy: RouteStrategy,
    model: &str,
    request_json: &Value,
    api_key_id: &str,
    request_seed: &str,
) -> String {
    match strategy {
        RouteStrategy::Priority => model.to_string(),
        RouteStrategy::Weighted => format!("{model}:{request_seed}"),
        RouteStrategy::StickyByKey => {
            let sticky_value = request_json
                .get("client_metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| {
                    ["session_id", "thread_id", "turn_id"]
                        .into_iter()
                        .find_map(|key| metadata.get(key).and_then(Value::as_str))
                })
                .filter(|value| !value.is_empty())
                .unwrap_or(api_key_id);
            format!("{model}:{sticky_value}")
        }
    }
}

fn route_error(model: &str, error: RoutingError) -> ApiError {
    match error {
        RoutingError::ModelNotFound => ApiError::gateway(
            StatusCode::NOT_FOUND,
            format!("Model {model} is not configured"),
            "model_not_found",
        ),
        RoutingError::Storage(error) => {
            tracing::error!(?error, "routing storage error");
            ApiError::gateway(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway storage error",
                "gateway_internal_error",
            )
        }
    }
}

pub(super) fn unavailable_error(model: &str) -> ApiError {
    ApiError::gateway(
        StatusCode::BAD_GATEWAY,
        format!("No healthy upstream available for model {model}"),
        "upstream_unavailable",
    )
}
