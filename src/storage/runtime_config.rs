use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::config::{
    Config, RUNTIME_CONFIG_DESCRIPTORS, RouteStrategy, RuntimeConfig, RuntimeConfigKey,
    RuntimeConfigValue,
};

use super::db::now_string;

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct SystemConfig {
    pub route_strategy: Option<String>,
    pub default_request_timeout_ms: Option<i64>,
    pub max_request_body_bytes: Option<i64>,
    pub request_log_retention_days: Option<i64>,
    pub daily_usage_retention_days: Option<i64>,
    pub expose_debug_headers: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct SystemConfigPatch {
    pub route_strategy: ConfigPatchValue<RouteStrategy>,
    pub default_request_timeout_ms: ConfigPatchValue<i64>,
    pub max_request_body_bytes: ConfigPatchValue<i64>,
    pub request_log_retention_days: ConfigPatchValue<i64>,
    pub daily_usage_retention_days: ConfigPatchValue<i64>,
    pub expose_debug_headers: ConfigPatchValue<bool>,
}

#[derive(Clone, Debug, Default)]
pub enum ConfigPatchValue<T> {
    #[default]
    Missing,
    Clear,
    Set(T),
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolvedRuntimeConfig {
    pub effective: RuntimeConfig,
    pub database: SystemConfig,
    pub fields: Vec<RuntimeConfigField>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub value_type: &'static str,
    pub validation: serde_json::Value,
    pub environment_variable: &'static str,
    pub unit: Option<&'static str>,
    pub value: serde_json::Value,
    pub source: &'static str,
    pub database_value: Option<serde_json::Value>,
    pub environment_value: Option<serde_json::Value>,
    pub default_value: serde_json::Value,
    pub editable: bool,
    pub live_reload: bool,
    pub requires_restart: bool,
}

impl SystemConfig {
    fn value(&self, key: RuntimeConfigKey) -> Option<RuntimeConfigValue> {
        match key {
            RuntimeConfigKey::RouteStrategy => self
                .route_strategy
                .as_deref()
                .and_then(|value| RouteStrategy::parse(value).ok())
                .map(RuntimeConfigValue::RouteStrategy),
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
            RuntimeConfigKey::ExposeDebugHeaders => self
                .expose_debug_headers
                .map(|value| RuntimeConfigValue::Boolean(value != 0)),
        }
    }
}

impl SystemConfigPatch {
    pub(crate) fn set(
        &mut self,
        key: RuntimeConfigKey,
        value: ConfigPatchValue<RuntimeConfigValue>,
    ) {
        match (key, value) {
            (RuntimeConfigKey::RouteStrategy, ConfigPatchValue::Missing) => {}
            (RuntimeConfigKey::RouteStrategy, ConfigPatchValue::Clear) => {
                self.route_strategy = ConfigPatchValue::Clear;
            }
            (
                RuntimeConfigKey::RouteStrategy,
                ConfigPatchValue::Set(RuntimeConfigValue::RouteStrategy(value)),
            ) => self.route_strategy = ConfigPatchValue::Set(value),
            (RuntimeConfigKey::DefaultRequestTimeoutMs, value) => {
                self.default_request_timeout_ms = integer_patch(value);
            }
            (RuntimeConfigKey::MaxRequestBodyBytes, value) => {
                self.max_request_body_bytes = integer_patch(value);
            }
            (RuntimeConfigKey::RequestLogRetentionDays, value) => {
                self.request_log_retention_days = integer_patch(value);
            }
            (RuntimeConfigKey::DailyUsageRetentionDays, value) => {
                self.daily_usage_retention_days = integer_patch(value);
            }
            (RuntimeConfigKey::ExposeDebugHeaders, ConfigPatchValue::Missing) => {}
            (RuntimeConfigKey::ExposeDebugHeaders, ConfigPatchValue::Clear) => {
                self.expose_debug_headers = ConfigPatchValue::Clear;
            }
            (
                RuntimeConfigKey::ExposeDebugHeaders,
                ConfigPatchValue::Set(RuntimeConfigValue::Boolean(value)),
            ) => self.expose_debug_headers = ConfigPatchValue::Set(value),
            _ => unreachable!("runtime configuration patch matches its descriptor type"),
        }
    }
}

fn integer_patch(value: ConfigPatchValue<RuntimeConfigValue>) -> ConfigPatchValue<i64> {
    match value {
        ConfigPatchValue::Missing => ConfigPatchValue::Missing,
        ConfigPatchValue::Clear => ConfigPatchValue::Clear,
        ConfigPatchValue::Set(RuntimeConfigValue::Integer(value)) => ConfigPatchValue::Set(value),
        ConfigPatchValue::Set(_) => {
            unreachable!("integer configuration patch has an integer value")
        }
    }
}

pub async fn get_system_config(pool: &SqlitePool) -> sqlx::Result<SystemConfig> {
    sqlx::query_as(
        "SELECT route_strategy, default_request_timeout_ms, max_request_body_bytes,
                request_log_retention_days, daily_usage_retention_days, expose_debug_headers,
                created_at, updated_at
         FROM system_config
         WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn upsert_system_config(
    pool: &SqlitePool,
    patch: &SystemConfigPatch,
) -> sqlx::Result<SystemConfig> {
    let mut conn = pool.acquire().await?;
    upsert_system_config_conn(&mut conn, patch).await
}

pub async fn upsert_system_config_conn(
    conn: &mut sqlx::SqliteConnection,
    patch: &SystemConfigPatch,
) -> sqlx::Result<SystemConfig> {
    let existing = get_system_config_conn(conn).await?;
    let now = now_string();
    let route_strategy = apply_config_patch(
        &patch.route_strategy,
        existing
            .route_strategy
            .as_deref()
            .and_then(|value| RouteStrategy::parse(value).ok()),
    )
    .map(|value| value.as_str().to_string());
    let default_request_timeout_ms = apply_config_patch(
        &patch.default_request_timeout_ms,
        existing.default_request_timeout_ms,
    );
    let max_request_body_bytes = apply_config_patch(
        &patch.max_request_body_bytes,
        existing.max_request_body_bytes,
    );
    let request_log_retention_days = apply_config_patch(
        &patch.request_log_retention_days,
        existing.request_log_retention_days,
    );
    let daily_usage_retention_days = apply_config_patch(
        &patch.daily_usage_retention_days,
        existing.daily_usage_retention_days,
    );
    let expose_debug_headers = apply_config_patch(
        &patch.expose_debug_headers,
        existing.expose_debug_headers.map(|v| v != 0),
    )
    .map(i64::from);

    sqlx::query(
        "UPDATE system_config
         SET route_strategy = ?,
             default_request_timeout_ms = ?,
             max_request_body_bytes = ?,
             request_log_retention_days = ?,
             daily_usage_retention_days = ?,
             expose_debug_headers = ?,
             updated_at = ?
         WHERE id = 1",
    )
    .bind(route_strategy)
    .bind(default_request_timeout_ms)
    .bind(max_request_body_bytes)
    .bind(request_log_retention_days)
    .bind(daily_usage_retention_days)
    .bind(expose_debug_headers)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    get_system_config_conn(conn).await
}

async fn get_system_config_conn(conn: &mut sqlx::SqliteConnection) -> sqlx::Result<SystemConfig> {
    sqlx::query_as(
        "SELECT route_strategy, default_request_timeout_ms, max_request_body_bytes,
                request_log_retention_days, daily_usage_retention_days, expose_debug_headers,
                created_at, updated_at
         FROM system_config
         WHERE id = 1",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn runtime_config(
    pool: &SqlitePool,
    config: &Config,
) -> sqlx::Result<ResolvedRuntimeConfig> {
    let database = get_system_config(pool).await?;
    Ok(resolve_runtime_config(config, database))
}

pub fn resolve_runtime_config(config: &Config, database: SystemConfig) -> ResolvedRuntimeConfig {
    let mut effective = crate::config::RuntimeEnvConfig::default().with_defaults();
    let fields = RUNTIME_CONFIG_DESCRIPTORS
        .iter()
        .map(|descriptor| {
            let environment_value = config.runtime_env.value(descriptor.key);
            let database_value = database.value(descriptor.key);
            let (value, source) = if let Some(value) = environment_value {
                (value, "environment")
            } else if let Some(value) = database_value {
                (value, "database")
            } else {
                (descriptor.default_value, "default")
            };
            effective.set(descriptor.key, value);
            RuntimeConfigField {
                key: descriptor.field_name,
                label: descriptor.display.label,
                value_type: descriptor.value_type.as_str(),
                validation: descriptor.validation_json(),
                environment_variable: descriptor.environment_variable,
                unit: descriptor.display.unit,
                value: value.to_json(),
                source,
                database_value: database_value.map(RuntimeConfigValue::to_json),
                environment_value: environment_value.map(RuntimeConfigValue::to_json),
                default_value: descriptor.default_value.to_json(),
                editable: descriptor.editable,
                live_reload: descriptor.live_reload,
                requires_restart: descriptor.requires_restart,
            }
        })
        .collect();

    ResolvedRuntimeConfig {
        effective,
        database,
        fields,
    }
}

fn apply_config_patch<T: Copy>(patch: &ConfigPatchValue<T>, current: Option<T>) -> Option<T> {
    match patch {
        ConfigPatchValue::Missing => current,
        ConfigPatchValue::Clear => None,
        ConfigPatchValue::Set(value) => Some(*value),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        RUNTIME_CONFIG_DESCRIPTORS, RuntimeConfigDescriptor, RuntimeConfigKey, RuntimeConfigValue,
    };

    use super::*;

    fn empty_database() -> SystemConfig {
        SystemConfig {
            route_strategy: None,
            default_request_timeout_ms: None,
            max_request_body_bytes: None,
            request_log_retention_days: None,
            daily_usage_retention_days: None,
            expose_debug_headers: None,
            created_at: "created".into(),
            updated_at: "updated".into(),
        }
    }

    fn database_value(key: RuntimeConfigKey) -> RuntimeConfigValue {
        match key {
            RuntimeConfigKey::RouteStrategy => {
                RuntimeConfigValue::RouteStrategy(RouteStrategy::Weighted)
            }
            RuntimeConfigKey::DefaultRequestTimeoutMs => RuntimeConfigValue::Integer(701),
            RuntimeConfigKey::MaxRequestBodyBytes => RuntimeConfigValue::Integer(702),
            RuntimeConfigKey::RequestLogRetentionDays => RuntimeConfigValue::Integer(703),
            RuntimeConfigKey::DailyUsageRetentionDays => RuntimeConfigValue::Integer(704),
            RuntimeConfigKey::ExposeDebugHeaders => RuntimeConfigValue::Boolean(true),
        }
    }

    fn environment_value(key: RuntimeConfigKey) -> RuntimeConfigValue {
        match key {
            RuntimeConfigKey::RouteStrategy => {
                RuntimeConfigValue::RouteStrategy(RouteStrategy::StickyByKey)
            }
            RuntimeConfigKey::DefaultRequestTimeoutMs => RuntimeConfigValue::Integer(801),
            RuntimeConfigKey::MaxRequestBodyBytes => RuntimeConfigValue::Integer(802),
            RuntimeConfigKey::RequestLogRetentionDays => RuntimeConfigValue::Integer(803),
            RuntimeConfigKey::DailyUsageRetentionDays => RuntimeConfigValue::Integer(804),
            RuntimeConfigKey::ExposeDebugHeaders => RuntimeConfigValue::Boolean(false),
        }
    }

    fn environment_raw(key: RuntimeConfigKey) -> &'static str {
        match key {
            RuntimeConfigKey::RouteStrategy => "sticky_by_key",
            RuntimeConfigKey::DefaultRequestTimeoutMs => "801",
            RuntimeConfigKey::MaxRequestBodyBytes => "802",
            RuntimeConfigKey::RequestLogRetentionDays => "803",
            RuntimeConfigKey::DailyUsageRetentionDays => "804",
            RuntimeConfigKey::ExposeDebugHeaders => "false",
        }
    }

    fn database_with(
        descriptor: &RuntimeConfigDescriptor,
        value: RuntimeConfigValue,
    ) -> SystemConfig {
        let mut database = empty_database();
        match (descriptor.key, value) {
            (RuntimeConfigKey::RouteStrategy, RuntimeConfigValue::RouteStrategy(value)) => {
                database.route_strategy = Some(value.as_str().into());
            }
            (RuntimeConfigKey::DefaultRequestTimeoutMs, RuntimeConfigValue::Integer(value)) => {
                database.default_request_timeout_ms = Some(value);
            }
            (RuntimeConfigKey::MaxRequestBodyBytes, RuntimeConfigValue::Integer(value)) => {
                database.max_request_body_bytes = Some(value);
            }
            (RuntimeConfigKey::RequestLogRetentionDays, RuntimeConfigValue::Integer(value)) => {
                database.request_log_retention_days = Some(value);
            }
            (RuntimeConfigKey::DailyUsageRetentionDays, RuntimeConfigValue::Integer(value)) => {
                database.daily_usage_retention_days = Some(value);
            }
            (RuntimeConfigKey::ExposeDebugHeaders, RuntimeConfigValue::Boolean(value)) => {
                database.expose_debug_headers = Some(i64::from(value));
            }
            _ => unreachable!("database value matches descriptor"),
        }
        database
    }

    fn field<'a>(
        resolved: &'a ResolvedRuntimeConfig,
        descriptor: &RuntimeConfigDescriptor,
    ) -> &'a RuntimeConfigField {
        resolved
            .fields
            .iter()
            .find(|field| field.key == descriptor.field_name)
            .unwrap()
    }

    #[test]
    fn runtime_resolution_and_display_contract_covers_every_field() {
        let default_config = Config::from_lookup(|_| None).unwrap();
        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            let resolved = resolve_runtime_config(&default_config, empty_database());
            let default_field = field(&resolved, descriptor);
            assert_eq!(default_field.source, "default", "{}", descriptor.field_name);
            assert_eq!(default_field.value, descriptor.default_value.to_json());

            let database = database_with(descriptor, database_value(descriptor.key));
            let resolved = resolve_runtime_config(&default_config, database.clone());
            let database_field = field(&resolved, descriptor);
            assert_eq!(
                database_field.source, "database",
                "{}",
                descriptor.field_name
            );
            assert_eq!(
                database_field.value,
                database_value(descriptor.key).to_json()
            );

            let environment_config = Config::from_lookup(|key| {
                (key == descriptor.environment_variable)
                    .then(|| environment_raw(descriptor.key).to_string())
            })
            .unwrap();
            let resolved = resolve_runtime_config(&environment_config, database);
            let environment_field = field(&resolved, descriptor);
            assert_eq!(
                environment_field.source, "environment",
                "{}",
                descriptor.field_name
            );
            assert_eq!(
                environment_field.value,
                environment_value(descriptor.key).to_json()
            );
            assert_eq!(environment_field.label, descriptor.display.label);
            assert_eq!(environment_field.value_type, descriptor.value_type.as_str());
            assert_eq!(environment_field.validation, descriptor.validation_json());
            assert_eq!(
                environment_field.environment_variable,
                descriptor.environment_variable
            );
            assert_eq!(environment_field.unit, descriptor.display.unit);
            assert_eq!(environment_field.editable, descriptor.editable);
            assert_eq!(environment_field.live_reload, descriptor.live_reload);
            assert_eq!(
                environment_field.requires_restart,
                descriptor.requires_restart
            );
        }
    }

    #[tokio::test]
    async fn runtime_clear_contract_covers_every_field() {
        let pool = crate::storage::connect_and_migrate("sqlite://:memory:")
            .await
            .unwrap();
        let default_config = Config::from_lookup(|_| None).unwrap();

        for descriptor in RUNTIME_CONFIG_DESCRIPTORS {
            let mut set = SystemConfigPatch::default();
            set.set(
                descriptor.key,
                ConfigPatchValue::Set(database_value(descriptor.key)),
            );
            let stored = upsert_system_config(&pool, &set).await.unwrap();
            assert_eq!(
                stored.value(descriptor.key),
                Some(database_value(descriptor.key)),
                "{} set",
                descriptor.field_name
            );

            let mut clear = SystemConfigPatch::default();
            clear.set(descriptor.key, ConfigPatchValue::Clear);
            let stored = upsert_system_config(&pool, &clear).await.unwrap();
            assert_eq!(
                stored.value(descriptor.key),
                None,
                "{} clear",
                descriptor.field_name
            );
            let resolved = resolve_runtime_config(&default_config, stored);
            let cleared_field = field(&resolved, descriptor);
            assert_eq!(cleared_field.source, "default", "{}", descriptor.field_name);
            assert_eq!(cleared_field.value, descriptor.default_value.to_json());
        }
    }
}
