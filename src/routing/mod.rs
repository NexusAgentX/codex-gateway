use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::config::{Config, RouteStrategy};

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
    #[serde(skip_serializing)]
    pub upstream_api_key_secret_version: i64,
    pub upstream_priority: i64,
    pub upstream_weight: i64,
    pub timeout_ms: i64,
    pub timeout_ms_is_explicit: i64,
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
    config: &Config,
    strategy: RouteStrategy,
    model_name: &str,
    sticky_key: Option<&str>,
) -> Result<RouteCandidate, RoutingError> {
    let candidates =
        route_candidates(pool, config, model_name, config.default_request_timeout_ms).await?;
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

    let generated_key = (strategy == RouteStrategy::Weighted && sticky_key.is_none())
        .then(|| uuid::Uuid::new_v4().to_string());
    let route_key = sticky_key
        .or(generated_key.as_deref())
        .unwrap_or(model_name);
    let candidate = order_candidates(&candidates, strategy, route_key)
        .into_iter()
        .next();
    candidate.ok_or(RoutingError::UpstreamUnavailable)
}

pub fn order_candidates(
    candidates: &[RouteCandidate],
    strategy: RouteStrategy,
    route_key: &str,
) -> Vec<RouteCandidate> {
    match strategy {
        RouteStrategy::Priority => candidates.to_vec(),
        RouteStrategy::Weighted | RouteStrategy::StickyByKey => {
            weighted_order(candidates, route_key)
        }
    }
}

pub async fn route_candidates(
    pool: &SqlitePool,
    config: &Config,
    model_name: &str,
    default_request_timeout_ms: i64,
) -> Result<Vec<RouteCandidate>, sqlx::Error> {
    let mut candidates: Vec<RouteCandidate> = sqlx::query_as(
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
            upstreams.api_key_secret_version AS upstream_api_key_secret_version,
            upstreams.priority AS upstream_priority,
            upstreams.weight AS upstream_weight,
            upstreams.timeout_ms AS timeout_ms,
            upstreams.timeout_ms_is_explicit AS timeout_ms_is_explicit,
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
    .await?;
    for candidate in &mut candidates {
        candidate.upstream_api_key = crate::secrets::decrypt_upstream_api_key(
            &config.app_secret,
            candidate.upstream_api_key_secret_version,
            &candidate.upstream_api_key,
        )
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
        if candidate.timeout_ms_is_explicit == 0 {
            candidate.timeout_ms = default_request_timeout_ms;
        }
    }
    Ok(candidates)
}

pub async fn model_exists(pool: &SqlitePool, model_name: &str) -> Result<bool, sqlx::Error> {
    let model_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM models WHERE public_name = ? AND enabled = 1")
            .bind(model_name)
            .fetch_optional(pool)
            .await?;
    Ok(model_exists.is_some())
}

fn weighted_order(candidates: &[RouteCandidate], key: &str) -> Vec<RouteCandidate> {
    let mut remaining = candidates.to_vec();
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut round = 0;
    while !remaining.is_empty() {
        let round_key = format!("{key}:{round}");
        let index = choose_weighted_index(&remaining, &round_key).unwrap_or(0);
        ordered.push(remaining.remove(index));
        round += 1;
    }
    ordered
}

fn choose_weighted_index(candidates: &[RouteCandidate], key: &str) -> Option<usize> {
    let total: i64 = candidates
        .iter()
        .map(|candidate| candidate.upstream_model_weight.max(1) * candidate.upstream_weight.max(1))
        .sum();
    if total <= 0 {
        return (!candidates.is_empty()).then_some(0);
    }

    let hash = stable_hash(key);
    let mut point = (hash % total as u64) as i64;
    for (index, candidate) in candidates.iter().enumerate() {
        let weight = candidate.upstream_model_weight.max(1) * candidate.upstream_weight.max(1);
        if point < weight {
            return Some(index);
        }
        point -= weight;
    }
    (!candidates.is_empty()).then_some(0)
}

fn stable_hash(key: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

    fn timeout_default() -> crate::storage::TimeoutPatchValue {
        crate::storage::TimeoutPatchValue::Default
    }

    #[tokio::test]
    async fn selects_lowest_priority_mapping() {
        let pool = pool().await;
        let config = Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite://:memory:".into(),
            app_secret: "test-secret".into(),
            secret_key_version: 1,
            public_url: "http://localhost".into(),
            cors_allowed_origins: vec!["http://localhost".into()],
            log_level: "info".into(),
            route_strategy: RouteStrategy::Priority,
            default_request_timeout_ms: crate::config::default_request_timeout_ms(),
            max_request_body_bytes: 10 * 1024 * 1024,
            health_checks_enabled: false,
            health_check_interval_ms: 30_000,
            request_log_retention_days: 90,
            daily_usage_retention_days: 730,
            retention_run_on_startup: true,
            expose_debug_headers: false,
            admin_email: None,
            admin_password: None,
            bootstrap_admin_key: None,
            runtime_env: Default::default(),
        };
        let slow = crate::storage::create_upstream(
            &pool,
            &config.app_secret,
            config.secret_key_version,
            &UpsertUpstream {
                name: "slow".into(),
                base_url: "http://slow".into(),
                api_key: "sk-slow".into(),
                enabled: Some(true),
                priority: Some(50),
                weight: Some(1),
                timeout_ms: timeout_default(),
                max_retries: None,
                health_check_path: None,
            },
        )
        .await
        .unwrap();
        let fast = crate::storage::create_upstream(
            &pool,
            &config.app_secret,
            config.secret_key_version,
            &UpsertUpstream {
                name: "fast".into(),
                base_url: "http://fast".into(),
                api_key: "sk-fast".into(),
                enabled: Some(true),
                priority: Some(1),
                weight: Some(1),
                timeout_ms: timeout_default(),
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

        let route = select_route(&pool, &config, RouteStrategy::Priority, "codex-mini", None)
            .await
            .unwrap();
        assert_eq!(route.upstream_model_name, "fast-model");
    }
}
