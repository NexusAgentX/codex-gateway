use reqwest::Url;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::{
    AppState,
    storage::{self, Upstream},
};

pub fn join_upstream_url(base_url: &str, canonical_path: &str) -> anyhow::Result<Url> {
    let base = base_url.trim_end_matches('/');
    let path = canonical_path.trim_start_matches('/');
    let url = format!("{base}/{path}");
    Ok(Url::parse(&url)?)
}

pub fn canonical_proxy_path(inbound_path: &str) -> Option<&'static str> {
    match inbound_path {
        "/responses" | "/v1/responses" => Some("/responses"),
        "/responses/compact" | "/v1/responses/compact" => Some("/responses/compact"),
        _ => None,
    }
}

pub async fn check_upstream_health(
    client: &reqwest::Client,
    pool: &sqlx::SqlitePool,
    app_secret: &str,
    upstream: &Upstream,
) -> anyhow::Result<String> {
    let url = join_upstream_url(&upstream.base_url, &upstream.health_check_path)?;
    let api_key = crate::secrets::decrypt_upstream_api_key(
        app_secret,
        upstream.api_key_secret_version,
        &upstream.api_key_ciphertext,
    )?;
    let result = client
        .get(url)
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_millis(
            upstream.timeout_ms.max(1) as u64
        ))
        .send()
        .await;

    let (status, sample) = match result {
        Ok(response) if response.status().is_success() => ("healthy", None),
        Ok(response) if response.status().is_server_error() => ("degraded", Some("http_5xx")),
        Ok(_) => ("down", Some("http_non_success")),
        Err(error) if error.is_timeout() => ("down", Some("upstream_timeout")),
        Err(error) if error.is_connect() => ("down", Some("upstream_error")),
        Err(_) => ("degraded", Some("upstream_error")),
    };
    storage::record_upstream_health(pool, &upstream.id, status, sample).await?;
    Ok(status.to_string())
}

pub fn spawn_health_worker(state: AppState) -> Option<JoinHandle<()>> {
    if !state.config.health_checks_enabled {
        return None;
    }
    Some(tokio::spawn(async move {
        run_health_worker(state).await;
    }))
}

async fn run_health_worker(state: AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(
        state.config.health_check_interval_ms,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        if let Err(error) = check_all_enabled_upstreams(&state).await {
            warn!(?error, "background upstream health check pass failed");
        }
    }
}

pub async fn check_all_enabled_upstreams(state: &AppState) -> anyhow::Result<usize> {
    let upstreams = storage::list_enabled_upstreams(&state.db).await?;
    let mut checked = 0;
    for upstream in upstreams {
        match check_upstream_health(&state.http, &state.db, &state.config.app_secret, &upstream)
            .await
        {
            Ok(status) => {
                checked += 1;
                debug!(upstream_id = %upstream.id, %status, "checked upstream health");
            }
            Err(error) => {
                warn!(?error, upstream_id = %upstream.id, "upstream health check failed");
                storage::record_upstream_health(
                    &state.db,
                    &upstream.id,
                    "degraded",
                    Some("health_check_error"),
                )
                .await?;
            }
        }
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_base_without_double_slashes() {
        assert_eq!(
            join_upstream_url("https://example.test/v1/", "/responses")
                .unwrap()
                .as_str(),
            "https://example.test/v1/responses"
        );
    }
}
