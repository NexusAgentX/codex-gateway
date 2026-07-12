use axum::{
    Json, Router,
    extract::{Extension, OriginalUri, Request, State},
    http::{HeaderMap, header},
    response::Response,
    routing::{get, post},
};
use serde_json::{Value, json};

use crate::{AppState, RequestId, auth, http_error::ApiError, storage};

mod attempt;
mod headers;
mod planning;
mod request;
mod settlement;
mod streaming;

pub use crate::upstream::headers::{
    authorization_header as upstream_authorization_header, forward_request_headers,
    forward_response_headers, is_hop_by_hop,
};
pub use request::sanitize_client_metadata;

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/responses", post(proxy_responses))
        .route("/v1/responses", post(proxy_responses))
        .route("/responses/compact", post(proxy_responses))
        .route("/v1/responses/compact", post(proxy_responses))
        .route("/v1/models", get(models))
        .with_state(state)
}

async fn authenticate_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<auth::AuthenticatedUser, ApiError> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    auth::authenticate_api_key_at(
        &state.db,
        &state.config.app_secret,
        authorization,
        state.clock.now(),
    )
    .await
    .map_err(ApiError::from_auth)
}

pub async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authenticate_api_key(&state, &headers).await?;
    let models = storage::list_visible_models(&state.db).await?;
    let data: Vec<Value> = models
        .into_iter()
        .map(|model| {
            json!({
                "id": model.public_name,
                "display_name": model.public_name,
                "object": "model",
                "type": "model",
                "created_at": model.created_at
            })
        })
        .collect();
    Ok(Json(json!({ "object": "list", "data": data })))
}

pub async fn proxy_responses(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Result<Response, ApiError> {
    let prepared = request::prepare(&state, request_id, uri, request).await?;
    let plan = planning::plan(&state, &prepared).await?;
    let admission = storage::admit_limited_request_with_clock(
        &state.db,
        &prepared.user.user_id,
        &prepared.user.api_key_id,
        state.clock.clone(),
    )
    .await?;
    let mut settlement = settlement::AdmissionSettlement::new(
        state.db.clone(),
        state.finalizations.clone(),
        state.clock.clone(),
        admission,
    );
    let mut retries_remaining = plan.max_retries();

    for (index, route) in plan.candidates.iter().enumerate() {
        if index > 0 {
            if retries_remaining <= 0 {
                break;
            }
            retries_remaining -= 1;
        }
        let can_retry =
            prepared.can_retry && index + 1 < plan.candidates.len() && retries_remaining > 0;
        let outcome =
            attempt::execute(&state, &prepared, &plan, route, index, retries_remaining).await;

        match outcome {
            attempt::AttemptOutcome::Success(success) => {
                let usage = success.record.usage.clone();
                settlement.set_total_tokens(usage.total_tokens);
                settlement::persist_attempt(&state.db, &state.finalizations, success.record).await;
                settlement.finalize(usage.total_tokens).await;
                return Ok(headers::unary_response(
                    success.response,
                    &prepared.request_id,
                    prepared.runtime.expose_debug_headers,
                    &plan.route_strategy,
                    &route.upstream_id,
                ));
            }
            attempt::AttemptOutcome::RetryableFailure(failure) => {
                settlement::persist_pre_attempt_health(
                    &state.db,
                    &state.finalizations,
                    failure.health,
                )
                .await;
                if let Some(record) = failure.record {
                    settlement::persist_attempt(&state.db, &state.finalizations, record).await;
                }
                if can_retry {
                    continue;
                }
                settlement.finalize(0).await;
                return failure.response.into_result().map(|response| {
                    headers::unary_response(
                        response,
                        &prepared.request_id,
                        prepared.runtime.expose_debug_headers,
                        &plan.route_strategy,
                        &route.upstream_id,
                    )
                });
            }
            attempt::AttemptOutcome::TerminalError(terminal) => {
                settlement::persist_pre_attempt_health(
                    &state.db,
                    &state.finalizations,
                    terminal.health,
                )
                .await;
                settlement.finalize(0).await;
                return Err(terminal.error);
            }
            attempt::AttemptOutcome::StreamingHandoff(stream) => {
                return Ok(streaming::response(
                    stream,
                    settlement,
                    prepared.runtime.expose_debug_headers,
                    &plan.route_strategy,
                ));
            }
        }
    }

    settlement.finalize(0).await;
    Err(planning::unavailable_error(&prepared.model))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sanitizes_client_metadata_without_raw_values() {
        let metadata = json!({
            "session_id": "sess-secret",
            "thread_id": "thread-secret",
            "x-codex-turn-metadata": "raw secret",
            "other": {"nested": "ignored"}
        });
        let sanitized = sanitize_client_metadata(Some(&metadata), "app-secret").unwrap();
        assert!(sanitized.contains("field_names"));
        assert!(sanitized.contains("session_id_hash"));
        assert!(sanitized.contains("thread_id_hash"));
        assert!(!sanitized.contains("sess-secret"));
        assert!(!sanitized.contains("thread-secret"));
        assert!(!sanitized.contains("raw secret"));
    }
}
