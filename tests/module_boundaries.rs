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
