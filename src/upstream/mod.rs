use reqwest::Url;

use crate::storage::{self, Upstream};

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

    let status = match result {
        Ok(response) if response.status().is_success() => "healthy",
        Ok(response) if response.status().is_server_error() => "degraded",
        Ok(_) => "down",
        Err(error) if error.is_timeout() || error.is_connect() => "down",
        Err(_) => "degraded",
    };
    storage::update_upstream_health(pool, &upstream.id, status).await?;
    Ok(status.to_string())
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
