#[test]
fn stage_two_reverse_dependencies_do_not_return() {
    let api = concat!(
        include_str!("../src/api/mod.rs"),
        include_str!("../src/api/auth.rs"),
        include_str!("../src/api/error.rs"),
        include_str!("../src/api/keys.rs"),
        include_str!("../src/api/limits.rs"),
        include_str!("../src/api/models.rs"),
        include_str!("../src/api/observability.rs"),
        include_str!("../src/api/settings.rs"),
        include_str!("../src/api/upstreams.rs"),
        include_str!("../src/api/users.rs"),
    );
    let proxy = concat!(
        include_str!("../src/proxy/mod.rs"),
        include_str!("../src/proxy/attempt.rs"),
        include_str!("../src/proxy/headers.rs"),
        include_str!("../src/proxy/planning.rs"),
        include_str!("../src/proxy/request.rs"),
        include_str!("../src/proxy/settlement.rs"),
        include_str!("../src/proxy/streaming.rs")
    );
    let auth = concat!(
        include_str!("../src/auth/mod.rs"),
        include_str!("../src/auth/persistence.rs")
    );

    assert!(
        !api.contains("crate::proxy") && !api.contains("proxy::"),
        "api must not register proxy handlers"
    );
    assert!(
        !proxy.contains("crate::api") && !proxy.contains("api::"),
        "proxy must not depend on api"
    );
    assert!(
        !auth.contains("crate::storage") && !auth.contains("storage::"),
        "auth must not depend on storage"
    );
}

#[test]
fn lib_composes_api_and_proxy_routers() {
    let lib = include_str!("../src/lib.rs");

    assert!(lib.contains("api::router(state.clone())"));
    assert!(lib.contains(".merge(proxy::router(state))"));
}

#[test]
fn stage_eight_narrows_internal_and_sensitive_rust_surfaces() {
    let lib = include_str!("../src/lib.rs");
    let routing = include_str!("../src/routing/mod.rs");
    let telemetry = include_str!("../src/telemetry/mod.rs");
    let storage = include_str!("../src/storage/mod.rs");

    for module in ["proxy", "routing", "telemetry"] {
        assert!(lib.contains(&format!("mod {module};")));
        assert!(!lib.contains(&format!("pub mod {module};")));
    }
    assert!(routing.contains("pub(crate) struct RouteCandidate"));
    assert!(!routing.contains("pub struct RouteCandidate"));
    assert!(!storage.contains("UpstreamRecord"));
    assert!(!storage.contains("UserCredentialsRecord"));
    assert!(telemetry.contains("-> Result<(), tracing_subscriber::util::TryInitError>"));
    assert!(!telemetry.contains("let _ = tracing_subscriber"));
    assert!(
        lib.contains("telemetry::init(&config.log_level).context(\"initializing telemetry\")?")
    );
}

#[test]
fn public_route_inventory_remains_at_stage_zero_baseline() {
    use std::collections::BTreeSet;

    let sources = [
        include_str!("../src/api/auth.rs"),
        include_str!("../src/api/keys.rs"),
        include_str!("../src/api/limits.rs"),
        include_str!("../src/api/models.rs"),
        include_str!("../src/api/observability.rs"),
        include_str!("../src/api/settings.rs"),
        include_str!("../src/api/upstreams.rs"),
        include_str!("../src/api/users.rs"),
        include_str!("../src/proxy/mod.rs"),
    ];
    let actual = sources
        .into_iter()
        .flat_map(route_inventory)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from(
        [
            "GET /api/admin/analytics",
            "GET /api/admin/api-keys",
            "GET /api/admin/api-keys/{id}/limits",
            "GET /api/admin/api-keys/{id}/usage",
            "GET /api/admin/limits",
            "GET /api/admin/metrics",
            "GET /api/admin/models",
            "GET /api/admin/models/{id}/mappings",
            "GET /api/admin/requests",
            "GET /api/admin/settings",
            "GET /api/admin/upstreams",
            "GET /api/admin/usage/daily",
            "GET /api/admin/usage/summary",
            "GET /api/admin/users",
            "GET /api/admin/users/{id}/limits",
            "GET /api/analytics",
            "GET /api/api-keys",
            "GET /api/api-keys/{id}/usage",
            "GET /api/limits",
            "GET /api/me",
            "GET /api/models",
            "GET /api/overview",
            "GET /api/requests",
            "GET /api/usage/daily",
            "GET /api/usage/summary",
            "GET /healthz",
            "GET /v1/models",
            "PATCH /api/admin/api-keys/{id}/limits",
            "PATCH /api/admin/limits/system",
            "PATCH /api/admin/model-mappings/{id}",
            "PATCH /api/admin/models/{id}",
            "PATCH /api/admin/settings",
            "PATCH /api/admin/upstreams/{id}",
            "PATCH /api/admin/users/{id}",
            "PATCH /api/admin/users/{id}/limits",
            "POST /api/admin/api-keys",
            "POST /api/admin/api-keys/{id}/disable",
            "POST /api/admin/api-keys/{id}/revoke",
            "POST /api/admin/model-mappings/{id}/disable",
            "POST /api/admin/models",
            "POST /api/admin/models/{id}/mappings",
            "POST /api/admin/retention/run",
            "POST /api/admin/upstreams",
            "POST /api/admin/upstreams/{id}/disable",
            "POST /api/admin/upstreams/{id}/health",
            "POST /api/admin/users",
            "POST /api/admin/users/{id}/password",
            "POST /api/api-keys",
            "POST /api/api-keys/{id}/disable",
            "POST /api/api-keys/{id}/revoke",
            "POST /api/login",
            "POST /responses",
            "POST /responses/compact",
            "POST /v1/responses",
            "POST /v1/responses/compact",
        ]
        .map(str::to_string),
    );

    assert_eq!(actual, expected);
    assert_eq!(
        actual
            .iter()
            .map(|route| route.split_once(' ').unwrap().1)
            .collect::<BTreeSet<_>>()
            .len(),
        46,
        "distinct path count changed"
    );
    assert_eq!(actual.len(), 55, "method/path count changed");
}

#[test]
fn api_domains_form_a_small_composition_facade() {
    let facade = include_str!("../src/api/mod.rs");
    let expected_modules = [
        "auth",
        "contracts",
        "error",
        "keys",
        "limits",
        "models",
        "observability",
        "settings",
        "upstreams",
        "users",
    ];
    let declared_modules = facade
        .lines()
        .filter_map(|line| line.strip_prefix("mod "))
        .map(|line| line.trim_end_matches(';'))
        .collect::<Vec<_>>();

    assert_eq!(declared_modules, expected_modules);
    assert!(facade.lines().count() < 40, "api facade grew handlers");
    assert!(!facade.contains("async fn"));
    assert!(!facade.contains(".route("));
    assert!(
        expected_modules
            .iter()
            .filter(|module| !matches!(**module, "contracts" | "error"))
            .all(|module| facade.contains(&format!(".merge({module}::router())")))
    );
    assert!(facade.contains("pub use auth::{authenticate, authenticate_api_key, require_admin};"));
    assert!(facade.contains("pub use error::ApiError;"));
    assert_eq!(
        include_str!("../src/api/error.rs").trim(),
        "pub use crate::http_error::ApiError;"
    );
}

#[test]
fn api_public_compatibility_paths_compile() {
    let _router: fn(codex_gateway::AppState) -> axum::Router = codex_gateway::api::router;
    let _ = std::any::type_name::<codex_gateway::api::ApiError>();
    let _ = codex_gateway::api::authenticate_api_key;
    let _ = codex_gateway::api::authenticate;
    let _ = codex_gateway::api::require_admin;
}

#[test]
fn admin_handlers_use_the_administrator_extractor() {
    let domains = [
        include_str!("../src/api/keys.rs"),
        include_str!("../src/api/limits.rs"),
        include_str!("../src/api/models.rs"),
        include_str!("../src/api/observability.rs"),
        include_str!("../src/api/settings.rs"),
        include_str!("../src/api/upstreams.rs"),
        include_str!("../src/api/users.rs"),
    ];

    for source in domains {
        assert!(
            !source.contains("require_admin"),
            "domain handler retained a manual administrator check"
        );
        for signature in async_function_signatures(source) {
            if signature.contains("fn admin_") {
                assert!(
                    signature.contains("Administrator"),
                    "admin handler is missing extractor: {signature}"
                );
            }
        }
    }
}

#[test]
fn storage_domains_form_an_explicit_compatibility_facade() {
    use std::collections::BTreeSet;

    let facade = include_str!("../src/storage/mod.rs");
    let domains = [
        ("analytics", include_str!("../src/storage/analytics.rs")),
        ("api_keys", include_str!("../src/storage/api_keys.rs")),
        ("audit", include_str!("../src/storage/audit.rs")),
        ("db", include_str!("../src/storage/db.rs")),
        ("limits", include_str!("../src/storage/limits.rs")),
        ("models", include_str!("../src/storage/models.rs")),
        (
            "request_logs",
            include_str!("../src/storage/request_logs.rs"),
        ),
        (
            "runtime_config",
            include_str!("../src/storage/runtime_config.rs"),
        ),
        ("upstreams", include_str!("../src/storage/upstreams.rs")),
        ("users", include_str!("../src/storage/users.rs")),
    ];

    let declared_domains = facade
        .lines()
        .filter_map(|line| line.strip_prefix("mod "))
        .map(|line| line.trim_end_matches(';'))
        .filter(|name| *name != "tests")
        .collect::<Vec<_>>();
    assert_eq!(
        declared_domains,
        domains.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
    assert!(domains.iter().all(|(_, source)| !source.trim().is_empty()));
    assert_eq!(facade.matches("pub use ").count(), domains.len());
    assert!(!facade.contains("pub struct "));
    assert!(!facade.contains("pub enum "));
    assert!(!facade.contains("pub async fn "));
    assert!(!facade.contains("sqlx::query"));
    assert!(!facade.contains("BEGIN IMMEDIATE"));

    let expected_public_items = BTreeSet::from([
        "AdminAuditInsert",
        "AdminAuditLog",
        "AdminLimitState",
        "AnalyticsDimensionShare",
        "AnalyticsLatencyBucket",
        "AnalyticsLatencyTrendBucket",
        "AnalyticsRequestBucket",
        "AnalyticsSnapshot",
        "AnalyticsTokenBucket",
        "AnalyticsUpstreamErrorRate",
        "AnalyticsUserErrorRate",
        "ApiKeySummary",
        "ApiKeyUsageSummary",
        "ConcurrencyState",
        "ConfigPatchValue",
        "CreateApiKey",
        "CreateUser",
        "DailyUsageFilters",
        "DailyUsageRow",
        "ErrorSummaryRow",
        "GatewayMetrics",
        "LatencyMetrics",
        "LimitAdmission",
        "LimitAdmissionError",
        "LimitBucketState",
        "LimitPatchValue",
        "LimitPolicy",
        "LimitPolicyPatch",
        "LimitRejection",
        "LimitSubjectState",
        "Model",
        "RequestLogFilters",
        "RequestLogInsert",
        "RequestLogRow",
        "ResetPassword",
        "ResolvedRuntimeConfig",
        "RetentionPolicy",
        "RetentionResult",
        "RuntimeConfigField",
        "SystemConfig",
        "SystemConfigPatch",
        "TimeoutPatchValue",
        "TokenUsageMetrics",
        "UpdateModel",
        "UpdateModelMapping",
        "UpdateUpstream",
        "UpdateUser",
        "UpsertModel",
        "UpsertModelMapping",
        "UpsertUpstream",
        "Upstream",
        "UpstreamHealthMetrics",
        "UpstreamModel",
        "UsageSummary",
        "UsageTotals",
        "User",
        "UserCredentials",
        "UserLimitState",
        "admin_limit_state",
        "admit_limited_request",
        "analytics_snapshot",
        "api_key_usage_summary",
        "apply_retention",
        "apply_retention_at",
        "apply_retention_conn",
        "connect_and_migrate",
        "create_api_key",
        "create_api_key_conn",
        "create_model",
        "create_model_conn",
        "create_upstream",
        "create_upstream_conn",
        "create_upstream_model",
        "create_upstream_model_conn",
        "ensure_bootstrap_admin",
        "ensure_user",
        "ensure_user_conn",
        "finalize_limit_admission",
        "find_user_credentials_by_email",
        "gateway_metrics",
        "get_api_key",
        "get_limit_policy",
        "get_model",
        "get_system_config",
        "get_upstream",
        "get_upstream_model",
        "get_user",
        "insert_admin_audit_log",
        "insert_request_log",
        "list_admin_audit_logs",
        "list_api_keys",
        "list_api_keys_for_user",
        "list_daily_usage",
        "list_daily_usage_filtered",
        "list_enabled_upstreams",
        "list_models",
        "list_request_logs",
        "list_request_logs_filtered",
        "list_upstream_models_for_model",
        "list_upstreams",
        "list_users",
        "list_visible_models",
        "mark_user_login",
        "now_string",
        "record_upstream_health",
        "record_upstream_health_conn",
        "reset_user_password",
        "reset_user_password_conn",
        "resolve_runtime_config",
        "runtime_config",
        "seed_bootstrap_admin",
        "set_api_key_status",
        "set_api_key_status_conn",
        "update_model",
        "update_model_conn",
        "update_upstream",
        "update_upstream_conn",
        "update_upstream_health",
        "update_upstream_model",
        "update_upstream_model_conn",
        "update_user",
        "update_user_conn",
        "upgrade_legacy_upstream_secrets",
        "upsert_limit_policy",
        "upsert_limit_policy_conn",
        "upsert_system_config",
        "upsert_system_config_conn",
        "usage_summary",
        "user_limit_state",
        "with_admin_audit",
    ]);
    let reexported_items = facade
        .split("pub use ")
        .skip(1)
        .map(|block| {
            block
                .split_once("::{")
                .expect("explicit grouped re-export")
                .1
        })
        .map(|block| block.split_once("};").expect("closed grouped re-export").0)
        .flat_map(|items| items.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>();
    assert_eq!(reexported_items, expected_public_items);
}

#[test]
fn stage_five_http_boundaries_use_api_owned_dtos() {
    let domains = [
        include_str!("../src/api/auth.rs"),
        include_str!("../src/api/keys.rs"),
        include_str!("../src/api/limits.rs"),
        include_str!("../src/api/models.rs"),
        include_str!("../src/api/observability.rs"),
        include_str!("../src/api/settings.rs"),
        include_str!("../src/api/upstreams.rs"),
        include_str!("../src/api/users.rs"),
    ];
    for source in domains {
        assert!(!source.contains("Json<storage::"));
        assert!(!source.contains("AdministratorJson<storage::"));
    }

    let contracts = include_str!("../src/api/contracts.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert!(contracts.contains("struct UpstreamResponse"));
    assert!(contracts.contains("struct RequestLogResponse"));
    assert!(!contracts.contains("password_hash"));
    assert!(!contracts.contains("key_hash"));
    assert!(!contracts.contains("api_key_ciphertext"));
}

#[test]
fn serialize_only_patch_types_keep_public_json_compatibility() {
    use codex_gateway::storage::{LimitPatchValue, LimitPolicyPatch, TimeoutPatchValue};
    use serde_json::json;

    assert_eq!(
        serde_json::to_value(LimitPolicyPatch::default()).unwrap(),
        json!({
            "request_quota": null,
            "request_window_seconds": null,
            "token_quota": null,
            "token_window_seconds": null,
            "rate_limit_requests": null,
            "rate_limit_window_seconds": null,
            "concurrency_limit": null
        })
    );
    assert_eq!(
        serde_json::to_value(LimitPatchValue::Set(7)).unwrap(),
        json!(7)
    );
    assert_eq!(
        serde_json::to_value(TimeoutPatchValue::Default).unwrap(),
        json!(null)
    );
    assert_eq!(
        serde_json::to_value(TimeoutPatchValue::Explicit(9)).unwrap(),
        json!(9)
    );
}

fn route_inventory(source: &str) -> Vec<String> {
    let mut routes = Vec::new();
    let mut remaining = source;
    while let Some(route_start) = remaining.find(".route(") {
        remaining = &remaining[route_start + ".route(".len()..];
        let quote_start = remaining.find('"').expect("route path starts");
        let after_quote = &remaining[quote_start + 1..];
        let quote_end = after_quote.find('"').expect("route path ends");
        let path = &after_quote[..quote_end];
        let expression = balanced_route_expression(&after_quote[quote_end + 1..]);
        for (needle, method) in [("get(", "GET"), ("post(", "POST"), ("patch(", "PATCH")] {
            if expression.contains(needle) {
                routes.push(format!("{method} {path}"));
            }
        }
        remaining = &after_quote[quote_end + 1..];
    }
    routes
}

fn balanced_route_expression(source: &str) -> &str {
    let mut depth = 1_i32;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[..index];
                }
            }
            _ => {}
        }
    }
    panic!("route expression closes")
}

fn async_function_signatures(source: &str) -> Vec<&str> {
    source
        .match_indices("async fn ")
        .map(|(start, _)| {
            let source = &source[start..];
            let end = source.find(" {").expect("function signature ends");
            &source[..end]
        })
        .collect()
}
