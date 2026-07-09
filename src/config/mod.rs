use std::env;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    pub app_secret: String,
    pub secret_key_version: i64,
    pub public_url: String,
    pub cors_allowed_origins: Vec<String>,
    pub log_level: String,
    pub route_strategy: RouteStrategy,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub bootstrap_admin_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStrategy {
    Priority,
    Weighted,
    StickyByKey,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind = lookup("CODEX_GATEWAY_BIND").unwrap_or_else(|| "127.0.0.1:8080".to_string());
        let database_url = lookup("CODEX_GATEWAY_DATABASE_URL")
            .unwrap_or_else(|| "sqlite://data/codex-gateway.db".to_string());
        let app_secret =
            lookup("CODEX_GATEWAY_APP_SECRET").unwrap_or_else(|| "dev-only-change-me".to_string());
        let environment = lookup("CODEX_GATEWAY_ENV")
            .or_else(|| lookup("RUST_ENV"))
            .or_else(|| lookup("ENV"))
            .unwrap_or_else(|| "development".to_string());
        validate_app_secret(
            &app_secret,
            &environment,
            lookup("CODEX_GATEWAY_REQUIRE_STRONG_SECRET").as_deref(),
        )?;
        let secret_key_version = lookup("CODEX_GATEWAY_SECRET_KEY_VERSION")
            .unwrap_or_else(|| "1".to_string())
            .parse::<i64>()
            .context("CODEX_GATEWAY_SECRET_KEY_VERSION must be an integer")?;
        if secret_key_version < 1 {
            bail!("CODEX_GATEWAY_SECRET_KEY_VERSION must be at least 1");
        }
        let public_url = lookup("CODEX_GATEWAY_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".to_string());
        let cors_allowed_origins = configured_origins(
            &public_url,
            lookup("CODEX_GATEWAY_PANEL_ORIGINS")
                .or_else(|| lookup("CODEX_GATEWAY_CORS_ALLOWED_ORIGINS"))
                .as_deref(),
        )?;
        let log_level = lookup("CODEX_GATEWAY_LOG_LEVEL").unwrap_or_else(|| "info".to_string());
        let route_strategy = match lookup("CODEX_GATEWAY_ROUTE_STRATEGY")
            .unwrap_or_else(|| "priority".to_string())
            .as_str()
        {
            "priority" => RouteStrategy::Priority,
            "weighted" => RouteStrategy::Weighted,
            "sticky_by_key" => RouteStrategy::StickyByKey,
            other => bail!("unsupported CODEX_GATEWAY_ROUTE_STRATEGY={other}"),
        };

        Ok(Self {
            bind,
            database_url,
            app_secret,
            secret_key_version,
            public_url,
            cors_allowed_origins,
            log_level,
            route_strategy,
            admin_email: lookup("CODEX_GATEWAY_ADMIN_EMAIL"),
            admin_password: lookup("CODEX_GATEWAY_ADMIN_PASSWORD"),
            bootstrap_admin_key: lookup("CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY"),
        })
    }
}

fn validate_app_secret(
    app_secret: &str,
    environment: &str,
    require_strong: Option<&str>,
) -> anyhow::Result<()> {
    let production_like = !matches!(
        environment.to_ascii_lowercase().as_str(),
        "development" | "dev" | "local" | "test"
    );
    let enforced = production_like || require_strong == Some("true");
    if !enforced {
        return Ok(());
    }
    if app_secret == "dev-only-change-me" || app_secret.len() < 32 {
        bail!("CODEX_GATEWAY_APP_SECRET must be set to at least 32 characters outside development");
    }
    Ok(())
}

fn configured_origins(
    public_url: &str,
    extra_origins: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut origins = Vec::new();
    origins.push(origin_from_url(public_url)?);
    if let Some(extra_origins) = extra_origins {
        for origin in extra_origins
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            origins.push(origin_from_url(origin)?);
        }
    }
    origins.sort();
    origins.dedup();
    Ok(origins)
}

fn origin_from_url(value: &str) -> anyhow::Result<String> {
    let parsed = Url::parse(value).with_context(|| format!("parsing configured origin {value}"))?;
    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        bail!("configured origin must use http or https: {value}");
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("configured origin must include host: {value}"))?;
    let origin = match parsed.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    };
    Ok(origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_defaults() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.bind, "127.0.0.1:8080");
        assert_eq!(config.database_url, "sqlite://data/codex-gateway.db");
        assert_eq!(config.route_strategy, RouteStrategy::Priority);
        assert_eq!(config.secret_key_version, 1);
        assert_eq!(config.cors_allowed_origins, vec!["http://localhost:8080"]);
    }

    #[test]
    fn rejects_unknown_strategy() {
        let result = Config::from_lookup(|key| {
            (key == "CODEX_GATEWAY_ROUTE_STRATEGY").then(|| "chaos".to_string())
        });
        assert!(result.is_err());
    }

    #[test]
    fn rejects_default_secret_outside_development() {
        let result = Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_ENV" => Some("production".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn accepts_strong_secret_and_configures_cors_origins() {
        let config = Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_ENV" => Some("production".to_string()),
            "CODEX_GATEWAY_APP_SECRET" => Some("0123456789abcdef0123456789abcdef".to_string()),
            "CODEX_GATEWAY_PUBLIC_URL" => Some("https://gateway.example.com/panel".to_string()),
            "CODEX_GATEWAY_PANEL_ORIGINS" => {
                Some("https://panel.example.com, http://localhost:5173".to_string())
            }
            "CODEX_GATEWAY_SECRET_KEY_VERSION" => Some("2".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.secret_key_version, 2);
        assert_eq!(
            config.cors_allowed_origins,
            vec![
                "http://localhost:5173",
                "https://gateway.example.com",
                "https://panel.example.com"
            ]
        );
    }
}
