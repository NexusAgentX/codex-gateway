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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeConfigKey {
    RouteStrategy,
    DefaultRequestTimeoutMs,
    MaxRequestBodyBytes,
    RequestLogRetentionDays,
    DailyUsageRetentionDays,
    ExposeDebugHeaders,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeConfigValue {
    RouteStrategy(RouteStrategy),
    Integer(i64),
    Boolean(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeConfigValueType {
    Enum,
    Integer,
    Boolean,
}

impl RuntimeConfigValueType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enum => "enum",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeConfigValidation {
    OneOf(&'static [&'static str]),
    Minimum(i64),
    Boolean,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfigDisplay {
    pub label: &'static str,
    pub unit: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfigDescriptor {
    pub key: RuntimeConfigKey,
    pub field_name: &'static str,
    pub environment_variable: &'static str,
    pub value_type: RuntimeConfigValueType,
    pub default_value: RuntimeConfigValue,
    pub validation: RuntimeConfigValidation,
    pub display: RuntimeConfigDisplay,
    pub editable: bool,
    pub live_reload: bool,
    pub requires_restart: bool,
}

const ROUTE_STRATEGY_VALUES: &[&str] = &["priority", "weighted", "sticky_by_key"];
pub const RUNTIME_CONFIG_PRECEDENCE: &str = "environment > database > default";
pub const DEFAULT_ROUTE_STRATEGY: RouteStrategy = RouteStrategy::Priority;
pub const DEFAULT_REQUEST_TIMEOUT_MS: i64 = 120_000;
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: i64 = 10 * 1024 * 1024;
pub const DEFAULT_REQUEST_LOG_RETENTION_DAYS: i64 = 90;
pub const DEFAULT_DAILY_USAGE_RETENTION_DAYS: i64 = 730;
pub const DEFAULT_EXPOSE_DEBUG_HEADERS: bool = false;

pub const RUNTIME_CONFIG_DESCRIPTORS: &[RuntimeConfigDescriptor] = &[
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::RouteStrategy,
        field_name: "route_strategy",
        environment_variable: "CODEX_GATEWAY_ROUTE_STRATEGY",
        value_type: RuntimeConfigValueType::Enum,
        default_value: RuntimeConfigValue::RouteStrategy(DEFAULT_ROUTE_STRATEGY),
        validation: RuntimeConfigValidation::OneOf(ROUTE_STRATEGY_VALUES),
        display: RuntimeConfigDisplay {
            label: "Default route strategy",
            unit: None,
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::DefaultRequestTimeoutMs,
        field_name: "default_request_timeout_ms",
        environment_variable: "CODEX_GATEWAY_DEFAULT_REQUEST_TIMEOUT_MS",
        value_type: RuntimeConfigValueType::Integer,
        default_value: RuntimeConfigValue::Integer(DEFAULT_REQUEST_TIMEOUT_MS),
        validation: RuntimeConfigValidation::Minimum(1),
        display: RuntimeConfigDisplay {
            label: "Default request timeout",
            unit: Some("ms"),
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::MaxRequestBodyBytes,
        field_name: "max_request_body_bytes",
        environment_variable: "CODEX_GATEWAY_MAX_REQUEST_BODY_BYTES",
        value_type: RuntimeConfigValueType::Integer,
        default_value: RuntimeConfigValue::Integer(DEFAULT_MAX_REQUEST_BODY_BYTES),
        validation: RuntimeConfigValidation::Minimum(1),
        display: RuntimeConfigDisplay {
            label: "Maximum request body size",
            unit: Some("bytes"),
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::RequestLogRetentionDays,
        field_name: "request_log_retention_days",
        environment_variable: "CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS",
        value_type: RuntimeConfigValueType::Integer,
        default_value: RuntimeConfigValue::Integer(DEFAULT_REQUEST_LOG_RETENTION_DAYS),
        validation: RuntimeConfigValidation::Minimum(0),
        display: RuntimeConfigDisplay {
            label: "Request log retention",
            unit: Some("days"),
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::DailyUsageRetentionDays,
        field_name: "daily_usage_retention_days",
        environment_variable: "CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS",
        value_type: RuntimeConfigValueType::Integer,
        default_value: RuntimeConfigValue::Integer(DEFAULT_DAILY_USAGE_RETENTION_DAYS),
        validation: RuntimeConfigValidation::Minimum(0),
        display: RuntimeConfigDisplay {
            label: "Daily usage retention",
            unit: Some("days"),
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
    RuntimeConfigDescriptor {
        key: RuntimeConfigKey::ExposeDebugHeaders,
        field_name: "expose_debug_headers",
        environment_variable: "CODEX_GATEWAY_EXPOSE_DEBUG_HEADERS",
        value_type: RuntimeConfigValueType::Boolean,
        default_value: RuntimeConfigValue::Boolean(DEFAULT_EXPOSE_DEBUG_HEADERS),
        validation: RuntimeConfigValidation::Boolean,
        display: RuntimeConfigDisplay {
            label: "Expose debug headers",
            unit: None,
        },
        editable: true,
        live_reload: true,
        requires_restart: false,
    },
];

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

impl RuntimeConfigDescriptor {
    pub fn parse_environment(self, value: &str) -> anyhow::Result<RuntimeConfigValue> {
        match self.value_type {
            RuntimeConfigValueType::Enum => RouteStrategy::parse(value)
                .map(RuntimeConfigValue::RouteStrategy)
                .with_context(|| format!("unsupported {}={value}", self.environment_variable)),
            RuntimeConfigValueType::Integer => {
                let parsed = value
                    .parse::<i64>()
                    .with_context(|| format!("{} must be an integer", self.environment_variable))?;
                self.validate_integer(parsed, self.environment_variable)
                    .map_err(anyhow::Error::msg)
                    .map(RuntimeConfigValue::Integer)
            }
            RuntimeConfigValueType::Boolean => {
                parse_bool(Some(value), false, self.environment_variable)
                    .map(RuntimeConfigValue::Boolean)
            }
        }
    }

    pub fn parse_json(self, value: &serde_json::Value) -> Result<RuntimeConfigValue, String> {
        match self.value_type {
            RuntimeConfigValueType::Enum => {
                let value = value
                    .as_str()
                    .ok_or_else(|| format!("{} must be a string or null", self.field_name))?;
                RouteStrategy::parse(value)
                    .map(RuntimeConfigValue::RouteStrategy)
                    .map_err(|_| {
                        format!(
                            "{} must be priority, weighted, or sticky_by_key",
                            self.field_name
                        )
                    })
            }
            RuntimeConfigValueType::Integer => {
                let parsed = value
                    .as_i64()
                    .ok_or_else(|| format!("{} must be an integer or null", self.field_name))?;
                self.validate_integer(parsed, self.field_name)
                    .map(RuntimeConfigValue::Integer)
            }
            RuntimeConfigValueType::Boolean => value
                .as_bool()
                .map(RuntimeConfigValue::Boolean)
                .ok_or_else(|| format!("{} must be a boolean or null", self.field_name)),
        }
    }

    pub fn validation_json(self) -> serde_json::Value {
        match self.validation {
            RuntimeConfigValidation::OneOf(values) => {
                serde_json::json!({ "allowed_values": values })
            }
            RuntimeConfigValidation::Minimum(minimum) => {
                serde_json::json!({ "minimum": minimum })
            }
            RuntimeConfigValidation::Boolean => serde_json::json!({}),
        }
    }

    fn validate_integer(self, value: i64, name: &str) -> Result<i64, String> {
        let RuntimeConfigValidation::Minimum(minimum) = self.validation else {
            return Ok(value);
        };
        if value >= minimum {
            return Ok(value);
        }
        let requirement = if minimum == 0 {
            "zero or greater".to_string()
        } else {
            format!("at least {minimum}")
        };
        Err(format!("{name} must be {requirement}"))
    }
}

impl RuntimeConfigValue {
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Self::RouteStrategy(value) => serde_json::json!(value.as_str()),
            Self::Integer(value) => serde_json::json!(value),
            Self::Boolean(value) => serde_json::json!(value),
        }
    }

    fn route_strategy(self) -> RouteStrategy {
        let Self::RouteStrategy(value) = self else {
            unreachable!("route strategy descriptor has a route strategy default")
        };
        value
    }

    fn integer(self) -> i64 {
        let Self::Integer(value) = self else {
            unreachable!("integer descriptor has an integer default")
        };
        value
    }

    fn boolean(self) -> bool {
        let Self::Boolean(value) = self else {
            unreachable!("boolean descriptor has a boolean default")
        };
        value
    }
}

pub fn runtime_config_descriptor(key: RuntimeConfigKey) -> &'static RuntimeConfigDescriptor {
    RUNTIME_CONFIG_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.key == key)
        .expect("every runtime configuration key has a descriptor")
}

pub fn runtime_config_descriptor_by_name(
    field_name: &str,
) -> Option<&'static RuntimeConfigDescriptor> {
    RUNTIME_CONFIG_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.field_name == field_name)
}

pub fn default_request_timeout_ms() -> i64 {
    DEFAULT_REQUEST_TIMEOUT_MS
}

impl RuntimeEnvConfig {
    fn from_lookup(lookup: &mut impl FnMut(&str) -> Option<String>) -> anyhow::Result<Self> {
        let mut values = Self::default();
        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            let Some(raw) = lookup(descriptor.environment_variable) else {
                continue;
            };
            values.set(descriptor.key, descriptor.parse_environment(&raw)?);
        }
        Ok(values)
    }

    pub fn value(&self, key: RuntimeConfigKey) -> Option<RuntimeConfigValue> {
        match key {
            RuntimeConfigKey::RouteStrategy => {
                self.route_strategy.map(RuntimeConfigValue::RouteStrategy)
            }
            RuntimeConfigKey::DefaultRequestTimeoutMs => self
                .default_request_timeout_ms
                .map(RuntimeConfigValue::Integer),
            RuntimeConfigKey::MaxRequestBodyBytes => {
                self.max_request_body_bytes.map(RuntimeConfigValue::Integer)
            }
            RuntimeConfigKey::RequestLogRetentionDays => self
                .request_log_retention_days
                .map(RuntimeConfigValue::Integer),
            RuntimeConfigKey::DailyUsageRetentionDays => self
                .daily_usage_retention_days
                .map(RuntimeConfigValue::Integer),
            RuntimeConfigKey::ExposeDebugHeaders => {
                self.expose_debug_headers.map(RuntimeConfigValue::Boolean)
            }
        }
    }

    fn set(&mut self, key: RuntimeConfigKey, value: RuntimeConfigValue) {
        match (key, value) {
            (RuntimeConfigKey::RouteStrategy, RuntimeConfigValue::RouteStrategy(value)) => {
                self.route_strategy = Some(value);
            }
            (RuntimeConfigKey::DefaultRequestTimeoutMs, RuntimeConfigValue::Integer(value)) => {
                self.default_request_timeout_ms = Some(value);
            }
            (RuntimeConfigKey::MaxRequestBodyBytes, RuntimeConfigValue::Integer(value)) => {
                self.max_request_body_bytes = Some(value);
            }
            (RuntimeConfigKey::RequestLogRetentionDays, RuntimeConfigValue::Integer(value)) => {
                self.request_log_retention_days = Some(value);
            }
            (RuntimeConfigKey::DailyUsageRetentionDays, RuntimeConfigValue::Integer(value)) => {
                self.daily_usage_retention_days = Some(value);
            }
            (RuntimeConfigKey::ExposeDebugHeaders, RuntimeConfigValue::Boolean(value)) => {
                self.expose_debug_headers = Some(value);
            }
            _ => unreachable!("runtime configuration value type matches its descriptor"),
        }
    }

    pub(crate) fn with_defaults(&self) -> RuntimeConfig {
        let mut config = RuntimeConfig {
            route_strategy: runtime_config_descriptor(RuntimeConfigKey::RouteStrategy)
                .default_value
                .route_strategy(),
            default_request_timeout_ms: default_request_timeout_ms(),
            max_request_body_bytes: runtime_config_descriptor(
                RuntimeConfigKey::MaxRequestBodyBytes,
            )
            .default_value
            .integer(),
            request_log_retention_days: runtime_config_descriptor(
                RuntimeConfigKey::RequestLogRetentionDays,
            )
            .default_value
            .integer(),
            daily_usage_retention_days: runtime_config_descriptor(
                RuntimeConfigKey::DailyUsageRetentionDays,
            )
            .default_value
            .integer(),
            expose_debug_headers: runtime_config_descriptor(RuntimeConfigKey::ExposeDebugHeaders)
                .default_value
                .boolean(),
        };
        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            config.set(
                descriptor.key,
                self.value(descriptor.key)
                    .unwrap_or(descriptor.default_value),
            );
        }
        config
    }
}

impl RuntimeConfig {
    pub(crate) fn set(&mut self, key: RuntimeConfigKey, value: RuntimeConfigValue) {
        match (key, value) {
            (RuntimeConfigKey::RouteStrategy, RuntimeConfigValue::RouteStrategy(value)) => {
                self.route_strategy = value;
            }
            (RuntimeConfigKey::DefaultRequestTimeoutMs, RuntimeConfigValue::Integer(value)) => {
                self.default_request_timeout_ms = value;
            }
            (RuntimeConfigKey::MaxRequestBodyBytes, RuntimeConfigValue::Integer(value)) => {
                self.max_request_body_bytes = value;
            }
            (RuntimeConfigKey::RequestLogRetentionDays, RuntimeConfigValue::Integer(value)) => {
                self.request_log_retention_days = value;
            }
            (RuntimeConfigKey::DailyUsageRetentionDays, RuntimeConfigValue::Integer(value)) => {
                self.daily_usage_retention_days = value;
            }
            (RuntimeConfigKey::ExposeDebugHeaders, RuntimeConfigValue::Boolean(value)) => {
                self.expose_debug_headers = value;
            }
            _ => unreachable!("runtime configuration value type matches its descriptor"),
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
        let runtime_env = RuntimeEnvConfig::from_lookup(&mut lookup)?;
        let runtime_defaults = runtime_env.with_defaults();
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
        let retention_run_on_startup = parse_bool(
            lookup("CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP").as_deref(),
            true,
            "CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP",
        )?;
        Ok(Self {
            bind,
            database_url,
            app_secret,
            secret_key_version,
            public_url,
            cors_allowed_origins,
            log_level,
            route_strategy: runtime_defaults.route_strategy,
            default_request_timeout_ms: runtime_defaults.default_request_timeout_ms,
            max_request_body_bytes: runtime_defaults.max_request_body_bytes,
            health_checks_enabled,
            health_check_interval_ms,
            request_log_retention_days: runtime_defaults.request_log_retention_days,
            daily_usage_retention_days: runtime_defaults.daily_usage_retention_days,
            retention_run_on_startup,
            expose_debug_headers: runtime_defaults.expose_debug_headers,
            admin_email: lookup("CODEX_GATEWAY_ADMIN_EMAIL"),
            admin_password: lookup("CODEX_GATEWAY_ADMIN_PASSWORD"),
            bootstrap_admin_key: lookup("CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY"),
            runtime_env,
        })
    }
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
    use std::collections::HashMap;

    use super::*;

    fn valid_environment_value(key: RuntimeConfigKey) -> &'static str {
        match key {
            RuntimeConfigKey::RouteStrategy => "weighted",
            RuntimeConfigKey::DefaultRequestTimeoutMs => "321",
            RuntimeConfigKey::MaxRequestBodyBytes => "654",
            RuntimeConfigKey::RequestLogRetentionDays => "0",
            RuntimeConfigKey::DailyUsageRetentionDays => "42",
            RuntimeConfigKey::ExposeDebugHeaders => "true",
        }
    }

    fn valid_runtime_value(key: RuntimeConfigKey) -> RuntimeConfigValue {
        match key {
            RuntimeConfigKey::RouteStrategy => {
                RuntimeConfigValue::RouteStrategy(RouteStrategy::Weighted)
            }
            RuntimeConfigKey::DefaultRequestTimeoutMs => RuntimeConfigValue::Integer(321),
            RuntimeConfigKey::MaxRequestBodyBytes => RuntimeConfigValue::Integer(654),
            RuntimeConfigKey::RequestLogRetentionDays => RuntimeConfigValue::Integer(0),
            RuntimeConfigKey::DailyUsageRetentionDays => RuntimeConfigValue::Integer(42),
            RuntimeConfigKey::ExposeDebugHeaders => RuntimeConfigValue::Boolean(true),
        }
    }

    fn invalid_environment_value(key: RuntimeConfigKey) -> &'static str {
        match key {
            RuntimeConfigKey::RouteStrategy => "random",
            RuntimeConfigKey::DefaultRequestTimeoutMs | RuntimeConfigKey::MaxRequestBodyBytes => {
                "0"
            }
            RuntimeConfigKey::RequestLogRetentionDays
            | RuntimeConfigKey::DailyUsageRetentionDays => "-1",
            RuntimeConfigKey::ExposeDebugHeaders => "maybe",
        }
    }

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
    fn runtime_descriptor_parsing_contract_covers_every_field() {
        assert_eq!(RUNTIME_CONFIG_DESCRIPTORS.len(), 6);
        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            let expected = valid_runtime_value(descriptor.key);
            assert_eq!(
                descriptor
                    .parse_environment(valid_environment_value(descriptor.key))
                    .unwrap(),
                expected,
                "{} environment parsing",
                descriptor.field_name
            );
            assert_eq!(
                descriptor.parse_json(&expected.to_json()).unwrap(),
                expected,
                "{} JSON parsing",
                descriptor.field_name
            );
            assert!(
                descriptor
                    .parse_environment(invalid_environment_value(descriptor.key))
                    .is_err(),
                "{} rejects invalid environment input",
                descriptor.field_name
            );
            assert!(
                descriptor.parse_json(&serde_json::json!([])).is_err(),
                "{} rejects the wrong JSON type",
                descriptor.field_name
            );
        }
    }

    #[test]
    fn runtime_environment_is_looked_up_and_parsed_once_per_field() {
        let mut lookup_counts = HashMap::<String, usize>::new();
        let config = Config::from_lookup(|key| {
            *lookup_counts.entry(key.to_string()).or_default() += 1;
            runtime_config_descriptor_by_name(
                RUNTIME_CONFIG_DESCRIPTORS
                    .iter()
                    .find(|descriptor| descriptor.environment_variable == key)?
                    .field_name,
            )
            .map(|descriptor| valid_environment_value(descriptor.key).to_string())
        })
        .unwrap();

        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            assert_eq!(
                lookup_counts.get(descriptor.environment_variable),
                Some(&1),
                "{} lookup count",
                descriptor.field_name
            );
            assert_eq!(
                config.runtime_env.value(descriptor.key),
                Some(valid_runtime_value(descriptor.key)),
                "{} parsed runtime value",
                descriptor.field_name
            );
        }
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
