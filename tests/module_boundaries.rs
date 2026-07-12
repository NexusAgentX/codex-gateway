#[test]
fn stage_two_reverse_dependencies_do_not_return() {
    let api = include_str!("../src/api/mod.rs");
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
fn public_route_inventory_remains_at_stage_zero_baseline() {
    let api_router = router_definition(include_str!("../src/api/mod.rs"));
    let proxy_router = router_definition(include_str!("../src/proxy/mod.rs"));
    let routers = [api_router, proxy_router];

    let route_count = routers
        .iter()
        .map(|source| source.matches(".route(").count())
        .sum::<usize>();
    let method_count = routers
        .iter()
        .map(|source| {
            source.matches("get(").count()
                + source.matches("post(").count()
                + source.matches("patch(").count()
        })
        .sum::<usize>();

    assert_eq!(route_count, 46, "distinct path count changed");
    assert_eq!(method_count, 55, "method/path count changed");
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

fn router_definition(source: &str) -> &str {
    let start = source
        .find("fn router")
        .expect("router function is present");
    let source = &source[start..];
    let end = source
        .find(".with_state(state)")
        .expect("router applies application state");
    &source[..end]
}
