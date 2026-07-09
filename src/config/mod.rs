use std::env;

use anyhow::bail;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    pub app_secret: String,
    pub public_url: String,
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
        let public_url = lookup("CODEX_GATEWAY_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".to_string());
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

        if lookup("CODEX_GATEWAY_REQUIRE_STRONG_SECRET").as_deref() == Some("true")
            && app_secret == "dev-only-change-me"
        {
            bail!("CODEX_GATEWAY_APP_SECRET must be set when strong secret enforcement is enabled");
        }

        Ok(Self {
            bind,
            database_url,
            app_secret,
            public_url,
            log_level,
            route_strategy,
            admin_email: lookup("CODEX_GATEWAY_ADMIN_EMAIL"),
            admin_password: lookup("CODEX_GATEWAY_ADMIN_PASSWORD"),
            bootstrap_admin_key: lookup("CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY"),
        })
    }
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
    }

    #[test]
    fn rejects_unknown_strategy() {
        let result = Config::from_lookup(|key| {
            (key == "CODEX_GATEWAY_ROUTE_STRATEGY").then(|| "chaos".to_string())
        });
        assert!(result.is_err());
    }
}
