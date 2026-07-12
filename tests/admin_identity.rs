use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use codex_gateway::{
    AppState, FinalizationTracker, JSON_BODY_LIMIT_BYTES, auth, build_app,
    config::{Config, RouteStrategy},
    storage::{self, CreateApiKey, CreateUser},
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::SqlitePool;
use tower::ServiceExt;

const JSON_CONTENT_TYPE: &str = "application/json";
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

struct AdminFixture {
    app: Router,
    pool: SqlitePool,
    admin_id: String,
    admin_key_id: String,
    admin_key: String,
    admin_panel: String,
    user_key_id: String,
    user_key: String,
    user_panel: String,
    expired_admin_key_id: String,
    expired_admin_key: String,
    expired_admin_panel: String,
    disabled_admin_key_id: String,
    disabled_admin_key: String,
    disabled_user_key_id: String,
    disabled_user_key: String,
    disabled_user_panel: String,
}

impl AdminFixture {
    fn seeded_api_keys(&self) -> [(&'static str, &str); 5] {
        [
            ("admin", &self.admin_key_id),
            ("user", &self.user_key_id),
            ("expired", &self.expired_admin_key_id),
            ("disabled-key", &self.disabled_admin_key_id),
            ("disabled-user", &self.disabled_user_key_id),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
enum CredentialKind {
    AdminPanel,
    AdminKey,
    Missing,
    InvalidPanel,
    InvalidKey,
    UserPanel,
    UserKey,
    ExpiredPanel,
    ExpiredKey,
    DisabledKey,
    DisabledUserPanel,
    DisabledUserKey,
}

impl CredentialKind {
    fn name(self) -> &'static str {
        match self {
            Self::AdminPanel => "admin-panel",
            Self::AdminKey => "admin-key",
            Self::Missing => "missing",
            Self::InvalidPanel => "invalid-panel",
            Self::InvalidKey => "invalid-key",
            Self::UserPanel => "user-panel",
            Self::UserKey => "user-key",
            Self::ExpiredPanel => "expired-panel",
            Self::ExpiredKey => "expired-key",
            Self::DisabledKey => "disabled-key",
            Self::DisabledUserPanel => "disabled-user-panel",
            Self::DisabledUserKey => "disabled-user-key",
        }
    }

    fn value(self, fixture: &AdminFixture) -> Option<&str> {
        match self {
            Self::AdminPanel => Some(&fixture.admin_panel),
            Self::AdminKey => Some(&fixture.admin_key),
            Self::Missing => None,
            Self::InvalidPanel => Some("cgw_panel_invalid.invalid"),
            Self::InvalidKey => Some("cgk_live_unknown_secret"),
            Self::UserPanel => Some(&fixture.user_panel),
            Self::UserKey => Some(&fixture.user_key),
            Self::ExpiredPanel => Some(&fixture.expired_admin_panel),
            Self::ExpiredKey => Some(&fixture.expired_admin_key),
            Self::DisabledKey => Some(&fixture.disabled_admin_key),
            Self::DisabledUserPanel => Some(&fixture.disabled_user_panel),
            Self::DisabledUserKey => Some(&fixture.disabled_user_key),
        }
    }

    fn is_admin(self) -> bool {
        matches!(self, Self::AdminPanel | Self::AdminKey)
    }

    fn active_api_key_id(self, fixture: &AdminFixture) -> Option<&str> {
        match self {
            Self::AdminKey => Some(&fixture.admin_key_id),
            Self::UserKey => Some(&fixture.user_key_id),
            _ => None,
        }
    }

    fn is_api_key_case(self) -> bool {
        matches!(
            self,
            Self::AdminKey
                | Self::InvalidKey
                | Self::UserKey
                | Self::ExpiredKey
                | Self::DisabledKey
                | Self::DisabledUserKey
        )
    }

    fn auth_error(self) -> ExpectedResponse {
        match self {
            Self::Missing | Self::InvalidPanel | Self::InvalidKey => gateway_error(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "invalid API key",
            ),
            Self::UserPanel | Self::UserKey => {
                gateway_error(StatusCode::FORBIDDEN, "forbidden", "admin role required")
            }
            Self::ExpiredPanel | Self::ExpiredKey => {
                gateway_error(StatusCode::FORBIDDEN, "expired_api_key", "expired API key")
            }
            Self::DisabledKey => gateway_error(
                StatusCode::FORBIDDEN,
                "disabled_api_key",
                "disabled API key",
            ),
            Self::DisabledUserPanel | Self::DisabledUserKey => {
                gateway_error(StatusCode::FORBIDDEN, "disabled_user", "disabled user")
            }
            Self::AdminPanel | Self::AdminKey => panic!("administrator has no auth error"),
        }
    }
}

const CREDENTIALS: [CredentialKind; 12] = [
    CredentialKind::AdminPanel,
    CredentialKind::AdminKey,
    CredentialKind::Missing,
    CredentialKind::InvalidPanel,
    CredentialKind::InvalidKey,
    CredentialKind::UserPanel,
    CredentialKind::UserKey,
    CredentialKind::ExpiredPanel,
    CredentialKind::ExpiredKey,
    CredentialKind::DisabledKey,
    CredentialKind::DisabledUserPanel,
    CredentialKind::DisabledUserKey,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyKind {
    Valid,
    Malformed,
    WrongContentType,
    MissingContentType,
    Empty,
    Oversized,
}

impl BodyKind {
    fn name(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Malformed => "malformed",
            Self::WrongContentType => "wrong-content-type",
            Self::MissingContentType => "missing-content-type",
            Self::Empty => "empty",
            Self::Oversized => "oversized",
        }
    }

    fn request_parts(self, valid_json: String) -> (Option<&'static str>, Body) {
        match self {
            Self::Valid => (Some(JSON_CONTENT_TYPE), Body::from(valid_json)),
            Self::Malformed => (Some(JSON_CONTENT_TYPE), Body::from("{")),
            Self::WrongContentType => (Some("text/plain"), Body::from(valid_json)),
            Self::MissingContentType => (None, Body::from(valid_json)),
            Self::Empty => (Some(JSON_CONTENT_TYPE), Body::empty()),
            Self::Oversized => (
                Some(JSON_CONTENT_TYPE),
                Body::from(format!(
                    r#"{{"padding":"{}"}}"#,
                    "x".repeat(JSON_BODY_LIMIT_BYTES)
                )),
            ),
        }
    }

    fn standard_rejection(self) -> Option<ExpectedResponse> {
        match self {
            Self::Valid => None,
            Self::Malformed => Some(text_error(
                StatusCode::BAD_REQUEST,
                "Failed to parse the request body as JSON: EOF while parsing an object at line 1 column 1",
            )),
            Self::WrongContentType | Self::MissingContentType => Some(text_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Expected request with `Content-Type: application/json`",
            )),
            Self::Empty => Some(text_error(
                StatusCode::BAD_REQUEST,
                "Failed to parse the request body as JSON: EOF while parsing a value at line 1 column 0",
            )),
            Self::Oversized => Some(text_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Failed to buffer the request body: length limit exceeded",
            )),
        }
    }

    fn settings_rejection(self) -> ExpectedResponse {
        match self {
            Self::Malformed | Self::Empty => gateway_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "request body must be JSON",
            ),
            Self::WrongContentType | Self::MissingContentType => gateway_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "invalid_request",
                "request body must be JSON",
            ),
            Self::Oversized => gateway_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_too_large",
                "request body exceeds configured maximum",
            ),
            Self::Valid => panic!("valid settings body is not a rejection case"),
        }
    }
}

const STANDARD_BODIES: [BodyKind; 6] = [
    BodyKind::Valid,
    BodyKind::Malformed,
    BodyKind::WrongContentType,
    BodyKind::MissingContentType,
    BodyKind::Empty,
    BodyKind::Oversized,
];

const REJECTED_BODIES: [BodyKind; 5] = [
    BodyKind::Malformed,
    BodyKind::WrongContentType,
    BodyKind::MissingContentType,
    BodyKind::Empty,
    BodyKind::Oversized,
];

#[derive(Debug)]
struct ExpectedResponse {
    status: StatusCode,
    content_type: &'static str,
    body: ExpectedBody,
}

#[derive(Debug)]
enum ExpectedBody {
    Json(Value),
    Text(&'static str),
}

struct CapturedResponse {
    status: StatusCode,
    content_type: String,
    body: Vec<u8>,
}

struct LastUsedSentinel {
    key_id: String,
    value: String,
}

#[tokio::test]
async fn administrator_json_write_matrix_preserves_responses_and_side_effects() {
    let fixture = admin_fixture().await;
    let initial_users = table_count(&fixture.pool, "users").await;
    let initial_audits = table_count(&fixture.pool, "admin_audit_logs").await;
    let mut successful_writes = 0_i64;

    for credential in CREDENTIALS {
        for body_kind in STANDARD_BODIES {
            let case = format!("{} / {}", credential.name(), body_kind.name());
            let email = format!(
                "matrix-{}-{}@example.com",
                credential.name(),
                body_kind.name()
            );
            let valid_json = json!({
                "email": email,
                "password": "password",
                "role": "user"
            })
            .to_string();
            let (content_type, body) = body_kind.request_parts(valid_json);
            let users_before = table_count(&fixture.pool, "users").await;
            let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
            let sentinels = reset_last_used_at_for_credential(
                &fixture,
                credential,
                &format!("standard:{}:{}", credential.name(), body_kind.name()),
            )
            .await;
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "POST",
                    "/api/admin/users",
                    credential.value(&fixture),
                    content_type,
                    body,
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            if let Some(sentinels) = &sentinels {
                let touched_key_id = (body_kind == BodyKind::Valid)
                    .then(|| credential.active_api_key_id(&fixture))
                    .flatten();
                assert_last_used_at(&fixture.pool, sentinels, touched_key_id, &case).await;
            }

            if body_kind == BodyKind::Valid && credential.is_admin() {
                assert_eq!(response.status, StatusCode::OK, "{case}");
                assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
                let response_json = response.json(&case);
                let id = response_json["id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{case}: missing user id"));
                assert_eq!(response_json, json!({ "id": id }), "{case}");
                assert_eq!(
                    table_count(&fixture.pool, "users").await,
                    users_before + 1,
                    "{case}"
                );
                assert_eq!(
                    table_count(&fixture.pool, "admin_audit_logs").await,
                    audits_before + 1,
                    "{case}"
                );
                assert_created_user_and_audit(&fixture, id, &email).await;
                successful_writes += 1;
            } else {
                let expected = body_kind
                    .standard_rejection()
                    .unwrap_or_else(|| credential.auth_error());
                assert_response(&response, &expected, &case);
                assert_eq!(
                    table_count(&fixture.pool, "users").await,
                    users_before,
                    "{case}: rejected request mutated users"
                );
                assert_eq!(
                    table_count(&fixture.pool, "admin_audit_logs").await,
                    audits_before,
                    "{case}: rejected request created an audit"
                );
                assert!(!user_email_exists(&fixture.pool, &email).await, "{case}");
            }
        }
    }

    assert_eq!(successful_writes, 2);
    assert_eq!(table_count(&fixture.pool, "users").await, initial_users + 2);
    assert_eq!(
        table_count(&fixture.pool, "admin_audit_logs").await,
        initial_audits + 2
    );
}

#[tokio::test]
async fn settings_json_matrix_preserves_auth_first_custom_errors_without_side_effects() {
    let fixture = admin_fixture().await;
    let initial_settings = settings_snapshot(&fixture.pool).await;
    let initial_audits = table_count(&fixture.pool, "admin_audit_logs").await;

    for credential in CREDENTIALS {
        for body_kind in REJECTED_BODIES {
            let case = format!("{} / {}", credential.name(), body_kind.name());
            let (content_type, body) = body_kind.request_parts("{}".to_string());
            let sentinels = reset_last_used_at_for_credential(
                &fixture,
                credential,
                &format!("settings:{}:{}", credential.name(), body_kind.name()),
            )
            .await;
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "PATCH",
                    "/api/admin/settings",
                    credential.value(&fixture),
                    content_type,
                    body,
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            if let Some(sentinels) = &sentinels {
                assert_last_used_at(
                    &fixture.pool,
                    sentinels,
                    credential.active_api_key_id(&fixture),
                    &case,
                )
                .await;
            }
            let expected = if credential.is_admin() {
                body_kind.settings_rejection()
            } else {
                credential.auth_error()
            };
            assert_response(&response, &expected, &case);
            assert_eq!(
                settings_snapshot(&fixture.pool).await,
                initial_settings,
                "{case}"
            );
            assert_eq!(
                table_count(&fixture.pool, "admin_audit_logs").await,
                initial_audits,
                "{case}: rejected settings request created an audit"
            );
        }
    }
}

#[tokio::test]
async fn path_and_query_rejections_precede_body_and_authorization() {
    let fixture = admin_fixture().await;
    let credentials = [
        CredentialKind::AdminPanel,
        CredentialKind::AdminKey,
        CredentialKind::InvalidKey,
        CredentialKind::UserKey,
    ];
    let path_bodies = [BodyKind::Valid, BodyKind::Malformed];
    let expected_path = text_error(
        StatusCode::BAD_REQUEST,
        "Invalid URL: Invalid UTF-8 in `id`",
    );
    let expected_query = text_error(
        StatusCode::BAD_REQUEST,
        "Failed to deserialize query string: limit: invalid digit found in string",
    );

    for credential in credentials {
        for body_kind in path_bodies {
            let case = format!("path / {} / {}", credential.name(), body_kind.name());
            let (content_type, body) =
                body_kind.request_parts(json!({ "role": "user" }).to_string());
            let users_before = table_count(&fixture.pool, "users").await;
            let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
            let sentinels = reset_last_used_at_for_credential(
                &fixture,
                credential,
                &format!("path:{}:{}", credential.name(), body_kind.name()),
            )
            .await;
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "PATCH",
                    "/api/admin/users/%FF",
                    credential.value(&fixture),
                    content_type,
                    body,
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            if let Some(sentinels) = &sentinels {
                assert_last_used_at(&fixture.pool, sentinels, None, &case).await;
            }
            assert_response(&response, &expected_path, &case);
            assert_eq!(
                table_count(&fixture.pool, "users").await,
                users_before,
                "{case}"
            );
            assert_eq!(
                table_count(&fixture.pool, "admin_audit_logs").await,
                audits_before,
                "{case}"
            );
        }

        let case = format!("query / {}", credential.name());
        let audits_before = table_count(&fixture.pool, "admin_audit_logs").await;
        let sentinels = reset_last_used_at_for_credential(
            &fixture,
            credential,
            &format!("query:{}", credential.name()),
        )
        .await;
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                "/api/admin/requests?limit=oops",
                credential.value(&fixture),
                None,
                Body::empty(),
            ))
            .await
            .unwrap();
        let response = capture(response).await;
        if let Some(sentinels) = &sentinels {
            assert_last_used_at(&fixture.pool, sentinels, None, &case).await;
        }
        assert_response(&response, &expected_query, &case);
        assert_eq!(
            table_count(&fixture.pool, "admin_audit_logs").await,
            audits_before,
            "{case}"
        );
    }
}

#[tokio::test]
async fn administrator_extractor_accepts_both_sources_on_domain_routes() {
    let fixture = admin_fixture().await;

    for credential in [CredentialKind::AdminKey, CredentialKind::AdminPanel] {
        for path in [
            "/api/admin/users",
            "/api/admin/api-keys",
            "/api/admin/upstreams",
            "/api/admin/models",
            "/api/admin/requests",
            "/api/admin/limits",
            "/api/admin/settings",
        ] {
            let case = format!("{} / {path}", credential.name());
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "GET",
                    path,
                    credential.value(&fixture),
                    None,
                    Body::empty(),
                ))
                .await
                .unwrap();
            let response = capture(response).await;
            assert_eq!(response.status, StatusCode::OK, "{case}");
            assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
            let _: Value = response.json(&case);
        }
    }
}

#[tokio::test]
async fn ordinary_panel_and_key_remain_valid_for_self_service() {
    let fixture = admin_fixture().await;

    for credential in [CredentialKind::UserPanel, CredentialKind::UserKey] {
        let case = credential.name();
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                "/api/me",
                credential.value(&fixture),
                None,
                Body::empty(),
            ))
            .await
            .unwrap();
        let response = capture(response).await;
        assert_eq!(response.status, StatusCode::OK, "{case}");
        assert_eq!(response.content_type, JSON_CONTENT_TYPE, "{case}");
        let body = response.json(case);
        assert_eq!(body["email"], "user@example.com", "{case}");
        assert_eq!(body["role"], "user", "{case}");
    }
}

impl CapturedResponse {
    fn json(&self, case: &str) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("{case}: invalid JSON response: {error}"))
    }
}

async fn capture(response: axum::response::Response) -> CapturedResponse {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<missing>")
        .to_string();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    CapturedResponse {
        status,
        content_type,
        body,
    }
}

fn assert_response(response: &CapturedResponse, expected: &ExpectedResponse, case: &str) {
    assert_eq!(response.status, expected.status, "{case}");
    assert_eq!(response.content_type, expected.content_type, "{case}");
    match &expected.body {
        ExpectedBody::Json(expected) => assert_eq!(response.json(case), *expected, "{case}"),
        ExpectedBody::Text(expected) => {
            assert_eq!(response.body.as_slice(), expected.as_bytes(), "{case}")
        }
    }
}

fn gateway_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
) -> ExpectedResponse {
    ExpectedResponse {
        status,
        content_type: JSON_CONTENT_TYPE,
        body: ExpectedBody::Json(json!({
            "error": {
                "message": message,
                "type": "gateway_error",
                "code": code,
                "details": null
            }
        })),
    }
}

fn text_error(status: StatusCode, body: &'static str) -> ExpectedResponse {
    ExpectedResponse {
        status,
        content_type: TEXT_CONTENT_TYPE,
        body: ExpectedBody::Text(body),
    }
}

fn request(
    method: &'static str,
    uri: &str,
    credential: Option<&str>,
    content_type: Option<&str>,
    body: Body,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(credential) = credential {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {credential}"));
    }
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder.body(body).unwrap()
}

async fn assert_created_user_and_audit(fixture: &AdminFixture, id: &str, email: &str) {
    let user = storage::get_user(&fixture.pool, id)
        .await
        .unwrap()
        .expect("created user exists");
    assert_eq!(user.email, email);
    assert_eq!(user.role, "user");
    assert_eq!(user.status, "active");

    let logs = storage::list_admin_audit_logs(&fixture.pool).await.unwrap();
    let audit = logs
        .iter()
        .find(|audit| audit.resource_id.as_deref() == Some(id))
        .expect("created user audit exists");
    assert_eq!(audit.actor_user_id, fixture.admin_id);
    assert_eq!(audit.actor_email, "admin@example.com");
    assert_eq!(audit.action, "create_user");
    assert_eq!(audit.resource_type, "user");
    assert_eq!(audit.status, "success");
    assert_eq!(
        serde_json::from_str::<Value>(audit.metadata_json.as_deref().unwrap()).unwrap(),
        json!({ "email": email, "role": "user" })
    );
}

async fn table_count(pool: &SqlitePool, table: &'static str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn user_email_exists(pool: &SqlitePool, email: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
        != 0
}

async fn settings_snapshot(pool: &SqlitePool) -> Value {
    serde_json::to_value(storage::get_system_config(pool).await.unwrap()).unwrap()
}

async fn reset_last_used_at_for_credential(
    fixture: &AdminFixture,
    credential: CredentialKind,
    case: &str,
) -> Option<Vec<LastUsedSentinel>> {
    if !credential.is_api_key_case() {
        return None;
    }
    Some(reset_last_used_at(fixture, case).await)
}

async fn reset_last_used_at(fixture: &AdminFixture, case: &str) -> Vec<LastUsedSentinel> {
    let mut sentinels = Vec::new();
    for (label, key_id) in fixture.seeded_api_keys() {
        let value = format!("sentinel:{case}:{label}");
        let updated = sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
            .bind(&value)
            .bind(key_id)
            .execute(&fixture.pool)
            .await
            .unwrap();
        assert_eq!(
            updated.rows_affected(),
            1,
            "{case}: missing seeded {label} key"
        );
        sentinels.push(LastUsedSentinel {
            key_id: key_id.to_string(),
            value,
        });
    }
    sentinels
}

async fn assert_last_used_at(
    pool: &SqlitePool,
    sentinels: &[LastUsedSentinel],
    touched_key_id: Option<&str>,
    case: &str,
) {
    let mut touched = 0;
    for sentinel in sentinels {
        let actual: Option<String> =
            sqlx::query_scalar("SELECT last_used_at FROM api_keys WHERE id = ?")
                .bind(&sentinel.key_id)
                .fetch_one(pool)
                .await
                .unwrap();
        if touched_key_id == Some(sentinel.key_id.as_str()) {
            let actual = actual.unwrap_or_else(|| panic!("{case}: touched key timestamp is null"));
            assert_ne!(
                actual, sentinel.value,
                "{case}: authentication did not touch key"
            );
            chrono::DateTime::parse_from_rfc3339(&actual)
                .unwrap_or_else(|error| panic!("{case}: invalid touched timestamp: {error}"));
            touched += 1;
        } else {
            assert_eq!(
                actual.as_deref(),
                Some(sentinel.value.as_str()),
                "{case}: unexpected key authentication side effect"
            );
        }
    }
    assert_eq!(touched, usize::from(touched_key_id.is_some()), "{case}");
}

async fn admin_fixture() -> AdminFixture {
    let pool = storage::connect_and_migrate("sqlite://:memory:")
        .await
        .unwrap();
    let config = test_config();
    let admin_id = create_user(&pool, "admin@example.com", "admin").await;
    let user_id = create_user(&pool, "user@example.com", "user").await;
    let disabled_user_id = create_user(&pool, "disabled@example.com", "admin").await;

    let (admin_key_id, admin_key) = create_key(&pool, &config, &admin_id, "admin", None).await;
    let (user_key_id, user_key) = create_key(&pool, &config, &user_id, "user", None).await;
    let (expired_admin_key_id, expired_admin_key) = create_key(
        &pool,
        &config,
        &admin_id,
        "expired",
        Some("2020-01-01T00:00:00Z"),
    )
    .await;
    let (disabled_key_row, disabled_admin_key) = storage::create_api_key(
        &pool,
        &config.app_secret,
        &admin_id,
        &CreateApiKey {
            name: "disabled".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    storage::set_api_key_status(&pool, &disabled_key_row.id, "disabled")
        .await
        .unwrap();
    let disabled_admin_key_id = disabled_key_row.id;
    let (disabled_user_key_id, disabled_user_key) =
        create_key(&pool, &config, &disabled_user_id, "disabled-user", None).await;

    let admin_panel = auth::generate_panel_token(&config.app_secret, &admin_id);
    let user_panel = auth::generate_panel_token(&config.app_secret, &user_id);
    let expired_admin_panel = expired_panel_token(&config.app_secret, &admin_id);
    let disabled_user_panel = auth::generate_panel_token(&config.app_secret, &disabled_user_id);
    storage::update_user(
        &pool,
        &disabled_user_id,
        &storage::UpdateUser {
            role: None,
            status: Some("disabled".into()),
            display_name: None,
        },
    )
    .await
    .unwrap();

    let state = AppState {
        config: std::sync::Arc::new(config),
        db: pool.clone(),
        http: reqwest::Client::new(),
        finalizations: FinalizationTracker::default(),
    };
    AdminFixture {
        app: build_app(state),
        pool,
        admin_id,
        admin_key_id,
        admin_key,
        admin_panel,
        user_key_id,
        user_key,
        user_panel,
        expired_admin_key_id,
        expired_admin_key,
        expired_admin_panel,
        disabled_admin_key_id,
        disabled_admin_key,
        disabled_user_key_id,
        disabled_user_key,
        disabled_user_panel,
    }
}

async fn create_user(pool: &SqlitePool, email: &str, role: &str) -> String {
    storage::ensure_user(
        pool,
        &CreateUser {
            email: email.into(),
            password: "password".into(),
            role: role.into(),
            display_name: None,
        },
    )
    .await
    .unwrap()
}

async fn create_key(
    pool: &SqlitePool,
    config: &Config,
    user_id: &str,
    name: &str,
    expires_at: Option<&str>,
) -> (String, String) {
    let (key, plaintext) = storage::create_api_key(
        pool,
        &config.app_secret,
        user_id,
        &CreateApiKey {
            name: name.into(),
            expires_at: expires_at.map(str::to_string),
        },
    )
    .await
    .unwrap();
    (key.id, plaintext)
}

fn expired_panel_token(app_secret: &str, user_id: &str) -> String {
    let payload = json!({
        "scope": "panel",
        "user_id": user_id,
        "session_id": "expired-session",
        "exp": 0
    });
    let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()).unwrap();
    mac.update(b"codex-gateway panel token v1");
    mac.update(payload_b64.as_bytes());
    format!(
        "cgw_panel_{payload_b64}.{}",
        hex::encode(mac.finalize().into_bytes())
    )
}

fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        database_url: "sqlite://:memory:".into(),
        app_secret: "test-secret".into(),
        secret_key_version: 1,
        public_url: "http://localhost".into(),
        cors_allowed_origins: vec!["http://localhost".into()],
        log_level: "info".into(),
        route_strategy: RouteStrategy::Priority,
        default_request_timeout_ms: 120_000,
        max_request_body_bytes: 10 * 1024 * 1024,
        health_checks_enabled: false,
        health_check_interval_ms: 30_000,
        request_log_retention_days: 90,
        daily_usage_retention_days: 730,
        retention_run_on_startup: true,
        expose_debug_headers: false,
        admin_email: None,
        admin_password: None,
        bootstrap_admin_key: None,
        runtime_env: Default::default(),
    }
}
