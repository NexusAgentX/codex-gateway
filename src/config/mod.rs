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
    pub default_request_timeout_ms: i64,
    pub max_request_body_bytes: i64,
    pub health_checks_enabled: bool,
    pub health_check_interval_ms: u64,
    pub request_log_retention_days: i64,
    pub daily_usage_retention_days: i64,
    pub retention_run_on_startup: bool,
    pub expose_debug_headers: bool,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub bootstrap_admin_key: Option<String>,
    pub runtime_env: RuntimeEnvConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStrategy {
    Priority,
    Weighted,
    StickyByKey,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RuntimeEnvConfig {
    pub route_strategy: Option<RouteStrategy>,
    pub default_request_timeout_ms: Option<i64>,
    pub max_request_body_bytes: Option<i64>,
    pub request_log_retention_days: Option<i64>,
    pub daily_usage_retention_days: Option<i64>,
    pub expose_debug_headers: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub route_strategy: RouteStrategy,
    pub default_request_timeout_ms: i64,
    pub max_request_body_bytes: i64,
    pub request_log_retention_days: i64,
    pub daily_usage_retention_days: i64,
    pub expose_debug_headers: bool,
}

pub const DEFAULT_ROUTE_STRATEGY: RouteStrategy = RouteStrategy::Priority;
pub const DEFAULT_REQUEST_TIMEOUT_MS: i64 = 120_000;
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: i64 = 10 * 1024 * 1024;
pub const DEFAULT_REQUEST_LOG_RETENTION_DAYS: i64 = 90;
pub const DEFAULT_DAILY_USAGE_RETENTION_DAYS: i64 = 730;
pub const DEFAULT_EXPOSE_DEBUG_HEADERS: bool = false;

impl RouteStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Priority => "priority",
            Self::Weighted => "weighted",
            Self::StickyByKey => "sticky_by_key",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "priority" => Ok(Self::Priority),
            "weighted" => Ok(Self::Weighted),
            "sticky_by_key" => Ok(Self::StickyByKey),
            other => bail!("unsupported route strategy={other}"),
        }
    }
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
        let route_strategy_env = lookup("CODEX_GATEWAY_ROUTE_STRATEGY");
        let route_strategy = match route_strategy_env.as_deref() {
            Some(value) => RouteStrategy::parse(value)
                .with_context(|| format!("unsupported CODEX_GATEWAY_ROUTE_STRATEGY={value}"))?,
            None => DEFAULT_ROUTE_STRATEGY,
        };
        let default_request_timeout_env = lookup("CODEX_GATEWAY_DEFAULT_REQUEST_TIMEOUT_MS");
        let default_request_timeout_ms = parse_positive_i64(
            default_request_timeout_env.as_deref(),
            DEFAULT_REQUEST_TIMEOUT_MS,
            "CODEX_GATEWAY_DEFAULT_REQUEST_TIMEOUT_MS",
        )?;
        let max_request_body_env = lookup("CODEX_GATEWAY_MAX_REQUEST_BODY_BYTES");
        let max_request_body_bytes = parse_positive_i64(
            max_request_body_env.as_deref(),
            DEFAULT_MAX_REQUEST_BODY_BYTES,
            "CODEX_GATEWAY_MAX_REQUEST_BODY_BYTES",
        )?;
        let health_checks_enabled = parse_bool(
            lookup("CODEX_GATEWAY_HEALTH_CHECKS_ENABLED").as_deref(),
            true,
            "CODEX_GATEWAY_HEALTH_CHECKS_ENABLED",
        )?;
        let health_check_interval_ms = lookup("CODEX_GATEWAY_HEALTH_CHECK_INTERVAL_MS")
            .unwrap_or_else(|| "30000".to_string())
            .parse::<u64>()
            .context("CODEX_GATEWAY_HEALTH_CHECK_INTERVAL_MS must be an integer")?;
        if health_check_interval_ms < 100 {
            bail!("CODEX_GATEWAY_HEALTH_CHECK_INTERVAL_MS must be at least 100");
        }
        let request_log_retention_env = lookup("CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS");
        let request_log_retention_days = parse_non_negative_i64(
            request_log_retention_env.as_deref(),
            DEFAULT_REQUEST_LOG_RETENTION_DAYS,
            "CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS",
        )?;
        let daily_usage_retention_env = lookup("CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS");
        let daily_usage_retention_days = parse_non_negative_i64(
            daily_usage_retention_env.as_deref(),
            DEFAULT_DAILY_USAGE_RETENTION_DAYS,
            "CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS",
        )?;
        let retention_run_on_startup = parse_bool(
            lookup("CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP").as_deref(),
            true,
            "CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP",
        )?;
        let expose_debug_headers_env = lookup("CODEX_GATEWAY_EXPOSE_DEBUG_HEADERS");
        let expose_debug_headers = parse_bool(
            expose_debug_headers_env.as_deref(),
            DEFAULT_EXPOSE_DEBUG_HEADERS,
            "CODEX_GATEWAY_EXPOSE_DEBUG_HEADERS",
        )?;
        let runtime_env = RuntimeEnvConfig {
            route_strategy: route_strategy_env
                .as_deref()
                .map(RouteStrategy::parse)
                .transpose()?,
            default_request_timeout_ms: default_request_timeout_env
                .as_deref()
                .map(|value| {
                    parse_positive_i64(
                        Some(value),
                        DEFAULT_REQUEST_TIMEOUT_MS,
                        "CODEX_GATEWAY_DEFAULT_REQUEST_TIMEOUT_MS",
                    )
                })
                .transpose()?,
            max_request_body_bytes: max_request_body_env
                .as_deref()
                .map(|value| {
                    parse_positive_i64(
                        Some(value),
                        DEFAULT_MAX_REQUEST_BODY_BYTES,
                        "CODEX_GATEWAY_MAX_REQUEST_BODY_BYTES",
                    )
                })
                .transpose()?,
            request_log_retention_days: request_log_retention_env
                .as_deref()
                .map(|value| {
                    parse_non_negative_i64(
                        Some(value),
                        DEFAULT_REQUEST_LOG_RETENTION_DAYS,
                        "CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS",
                    )
                })
                .transpose()?,
            daily_usage_retention_days: daily_usage_retention_env
                .as_deref()
                .map(|value| {
                    parse_non_negative_i64(
                        Some(value),
                        DEFAULT_DAILY_USAGE_RETENTION_DAYS,
                        "CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS",
                    )
                })
                .transpose()?,
            expose_debug_headers: expose_debug_headers_env
                .as_deref()
                .map(|value| {
                    parse_bool(
                        Some(value),
                        DEFAULT_EXPOSE_DEBUG_HEADERS,
                        "CODEX_GATEWAY_EXPOSE_DEBUG_HEADERS",
                    )
                })
                .transpose()?,
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
            default_request_timeout_ms,
            max_request_body_bytes,
            health_checks_enabled,
            health_check_interval_ms,
            request_log_retention_days,
            daily_usage_retention_days,
            retention_run_on_startup,
            expose_debug_headers,
            admin_email: lookup("CODEX_GATEWAY_ADMIN_EMAIL"),
            admin_password: lookup("CODEX_GATEWAY_ADMIN_PASSWORD"),
            bootstrap_admin_key: lookup("CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY"),
            runtime_env,
        })
    }
}

fn parse_positive_i64(value: Option<&str>, default: i64, name: &str) -> anyhow::Result<i64> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value
        .parse::<i64>()
        .with_context(|| format!("{name} must be an integer"))?;
    if parsed < 1 {
        bail!("{name} must be at least 1");
    }
    Ok(parsed)
}

fn parse_non_negative_i64(value: Option<&str>, default: i64, name: &str) -> anyhow::Result<i64> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value
        .parse::<i64>()
        .with_context(|| format!("{name} must be an integer"))?;
    if parsed < 0 {
        bail!("{name} must be zero or greater");
    }
    Ok(parsed)
}

fn parse_bool(value: Option<&str>, default: bool, name: &str) -> anyhow::Result<bool> {
    let Some(value) = value else {
        return Ok(default);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{name} must be true or false"),
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
        assert!(config.health_checks_enabled);
        assert_eq!(config.health_check_interval_ms, 30_000);
        assert_eq!(config.request_log_retention_days, 90);
        assert_eq!(config.daily_usage_retention_days, 730);
        assert!(config.retention_run_on_startup);
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

    #[test]
    fn configures_health_worker_controls() {
        let config = Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_HEALTH_CHECKS_ENABLED" => Some("false".to_string()),
            "CODEX_GATEWAY_HEALTH_CHECK_INTERVAL_MS" => Some("250".to_string()),
            _ => None,
        })
        .unwrap();
        assert!(!config.health_checks_enabled);
        assert_eq!(config.health_check_interval_ms, 250);
    }

    #[test]
    fn configures_retention_policy() {
        let config = Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS" => Some("14".to_string()),
            "CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS" => Some("365".to_string()),
            "CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP" => Some("false".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.request_log_retention_days, 14);
        assert_eq!(config.daily_usage_retention_days, 365);
        assert!(!config.retention_run_on_startup);
    }
}
