use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::config::RouteStrategy;

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct RouteCandidate {
    pub model_id: String,
    pub public_name: String,
    pub upstream_model_id: String,
    pub upstream_model_name: String,
    pub upstream_model_priority: i64,
    pub upstream_model_weight: i64,
    pub upstream_id: String,
    pub upstream_name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub upstream_api_key: String,
    pub upstream_priority: i64,
    pub upstream_weight: i64,
    pub timeout_ms: i64,
    pub max_retries: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("model not found")]
    ModelNotFound,
    #[error("no healthy upstream available")]
    UpstreamUnavailable,
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

pub async fn select_route(
    pool: &SqlitePool,
    strategy: RouteStrategy,
    model_name: &str,
    sticky_key: Option<&str>,
) -> Result<RouteCandidate, RoutingError> {
    let candidates = route_candidates(pool, model_name).await?;
    if candidates.is_empty() {
        let model_exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM models WHERE public_name = ? AND enabled = 1")
                .bind(model_name)
                .fetch_optional(pool)
                .await?;
        return if model_exists.is_some() {
            Err(RoutingError::UpstreamUnavailable)
        } else {
            Err(RoutingError::ModelNotFound)
        };
    }

    let candidate = match strategy {
        RouteStrategy::Priority => candidates.into_iter().next(),
        RouteStrategy::Weighted => choose_weighted(&candidates, sticky_key.unwrap_or(model_name)),
        RouteStrategy::StickyByKey => {
            choose_weighted(&candidates, sticky_key.unwrap_or(model_name))
        }
    };
    candidate.ok_or(RoutingError::UpstreamUnavailable)
}

pub async fn route_candidates(
    pool: &SqlitePool,
    model_name: &str,
) -> Result<Vec<RouteCandidate>, sqlx::Error> {
    sqlx::query_as(
        "SELECT
            models.id AS model_id,
            models.public_name AS public_name,
            upstream_models.id AS upstream_model_id,
            upstream_models.upstream_model_name AS upstream_model_name,
            upstream_models.priority AS upstream_model_priority,
            upstream_models.weight AS upstream_model_weight,
            upstreams.id AS upstream_id,
            upstreams.name AS upstream_name,
            upstreams.base_url AS base_url,
            upstreams.api_key_ciphertext AS upstream_api_key,
            upstreams.priority AS upstream_priority,
            upstreams.weight AS upstream_weight,
            upstreams.timeout_ms AS timeout_ms,
            upstreams.max_retries AS max_retries
         FROM models
         JOIN upstream_models ON upstream_models.model_id = models.id
         JOIN upstreams ON upstreams.id = upstream_models.upstream_id
         WHERE models.public_name = ?
           AND models.enabled = 1
           AND upstream_models.enabled = 1
           AND upstreams.enabled = 1
           AND upstreams.last_health_status != 'down'
         ORDER BY upstream_models.priority ASC, upstreams.priority ASC, upstream_models.id ASC",
    )
    .bind(model_name)
    .fetch_all(pool)
    .await
}

pub async fn model_exists(pool: &SqlitePool, model_name: &str) -> Result<bool, sqlx::Error> {
    let model_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM models WHERE public_name = ? AND enabled = 1")
            .bind(model_name)
            .fetch_optional(pool)
            .await?;
    Ok(model_exists.is_some())
}

fn choose_weighted(candidates: &[RouteCandidate], key: &str) -> Option<RouteCandidate> {
    let total: i64 = candidates
        .iter()
        .map(|candidate| candidate.upstream_model_weight.max(1) * candidate.upstream_weight.max(1))
        .sum();
    if total <= 0 {
        return candidates.first().cloned();
    }

    let mut hash = 0_u64;
    for byte in key.as_bytes() {
        hash = hash.wrapping_mul(16777619) ^ u64::from(*byte);
    }
    let mut point = (hash % total as u64) as i64;
    for candidate in candidates {
        let weight = candidate.upstream_model_weight.max(1) * candidate.upstream_weight.max(1);
        if point < weight {
            return Some(candidate.clone());
        }
        point -= weight;
    }
    candidates.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateUser, UpsertModel, UpsertModelMapping, UpsertUpstream};

    async fn pool() -> SqlitePool {
        let pool = crate::storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        crate::storage::ensure_user(
            &pool,
            &CreateUser {
                email: "a@example.com".to_string(),
                password: "password".to_string(),
                role: "admin".to_string(),
                display_name: None,
            },
        )
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn selects_lowest_priority_mapping() {
        let pool = pool().await;
        let slow = crate::storage::create_upstream(
            &pool,
            &UpsertUpstream {
                name: "slow".into(),
                base_url: "http://slow".into(),
                api_key: "sk-slow".into(),
                enabled: Some(true),
                priority: Some(50),
                weight: Some(1),
                timeout_ms: None,
                max_retries: None,
                health_check_path: None,
            },
        )
        .await
        .unwrap();
        let fast = crate::storage::create_upstream(
            &pool,
            &UpsertUpstream {
                name: "fast".into(),
                base_url: "http://fast".into(),
                api_key: "sk-fast".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
                timeout_ms: None,
                max_retries: None,
                health_check_path: None,
            },
        )
        .await
        .unwrap();
        crate::storage::create_model(
            &pool,
            &UpsertModel {
                public_name: "codex-mini".into(),
                description: None,
                enabled: Some(true),
                visible_to_users: Some(true),
                upstream_mappings: Some(vec![
                    UpsertModelMapping {
                        upstream_id: slow.id,
                        upstream_model_name: "slow-model".into(),
                        enabled: Some(true),
                        priority: Some(10),
                        weight: Some(1),
                    },
                    UpsertModelMapping {
                        upstream_id: fast.id,
                        upstream_model_name: "fast-model".into(),
                        enabled: Some(true),
                        priority: Some(1),
                        weight: Some(1),
                    },
                ]),
            },
        )
        .await
        .unwrap();

        let route = select_route(&pool, RouteStrategy::Priority, "codex-mini", None)
            .await
            .unwrap();
        assert_eq!(route.upstream_model_name, "fast-model");
    }
}
