#[path = "security/storage.rs"]
mod storage;
mod support;

use support::*;

#[tokio::test]
async fn cors_rejects_untrusted_origins_when_configured() {
    let (app, _) = test_app(None).await;

    let allowed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/me")
                .header(header::ORIGIN, "http://localhost")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        allowed
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "http://localhost"
    );

    let rejected = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/me")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        rejected
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[tokio::test]
async fn production_like_config_refuses_default_or_weak_secrets() {
    assert!(
        Config::from_lookup(|key| {
            (key == "CODEX_GATEWAY_ENV").then(|| "production".to_string())
        })
        .is_err()
    );

    assert!(
        Config::from_lookup(|key| match key {
            "CODEX_GATEWAY_ENV" => Some("production".to_string()),
            "CODEX_GATEWAY_APP_SECRET" => Some("short".to_string()),
            _ => None,
        })
        .is_err()
    );
}
