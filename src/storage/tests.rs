use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Duration, Utc};

use sqlx::{
    QueryBuilder, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use super::upstreams::record_upstream_health_at;
use super::*;
use super::{db::with_immediate_transaction, limits::admit_limited_request_in_tx};
use crate::clock::{Clock, SharedClock};

#[derive(Clone)]
struct ManualClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl ManualClock {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap();
        *now += duration;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
}

async fn single_connection_file_pool(filename: &str) -> (tempfile::TempDir, SqlitePool) {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_url = format!("sqlite://{}", temp_dir.path().join(filename).display());
    let options = SqliteConnectOptions::from_str(&database_url)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (temp_dir, pool)
}

async fn hold_only_connection(
    pool: &SqlitePool,
) -> (tokio::task::JoinHandle<()>, Arc<tokio::sync::Notify>) {
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let blocker_pool = pool.clone();
    let blocker_release = release.clone();
    let blocker = tokio::spawn(async move {
        with_immediate_transaction::<_, sqlx::Error, _>(&blocker_pool, move |_conn| {
            Box::pin(async move {
                locked_tx.send(()).unwrap();
                blocker_release.notified().await;
                Ok(())
            })
        })
        .await
        .unwrap();
    });
    locked_rx.await.unwrap();
    (blocker, release)
}

async fn seeded_limit_principal(pool: &SqlitePool, email: &str) -> (String, ApiKeySummary) {
    let user_id = ensure_user(
        pool,
        &CreateUser {
            email: email.into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (key, _) = create_api_key(
        pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "clock-test".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    (user_id, key)
}

#[tokio::test]
async fn cancelled_limit_admission_rolls_back_and_reuses_single_connection() {
    let (_temp_dir, pool) = single_connection_file_pool("cancelled-admission.db").await;
    assert_eq!(pool.size(), 1);
    sqlx::query("CREATE TEMP TABLE cancelled_connection_marker (id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    let user_id = ensure_user(
        &pool,
        &CreateUser {
            email: "cancelled-admission@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (key, _) = create_api_key(
        &pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "cancelled-admission".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let cancellation_gate = Arc::new(tokio::sync::Notify::new());
    let cancelled_pool = pool.clone();
    let cancelled_user_id = user_id.clone();
    let cancelled_key_id = key.id.clone();
    let cancelled_task = tokio::spawn({
        let cancellation_gate = cancellation_gate.clone();
        async move {
            with_immediate_transaction::<_, LimitAdmissionError, _>(&cancelled_pool, move |conn| {
                Box::pin(async move {
                    let admission =
                        admit_limited_request_in_tx(conn, &cancelled_user_id, &cancelled_key_id)
                            .await?;
                    started_tx.send(()).unwrap();
                    cancellation_gate.notified().await;
                    Ok(admission)
                })
            })
            .await
        }
    });

    started_rx.await.unwrap();
    cancelled_task.abort();
    assert!(cancelled_task.await.unwrap_err().is_cancelled());
    let connection_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_temp_master
             WHERE type = 'table' AND name = 'cancelled_connection_marker'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(connection_marker_count, 0);

    let usage_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_usage_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let inflight_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    let rate_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_rate_counters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((usage_count, inflight_count, rate_count), (0, 0, 0));

    let admission = admit_limited_request(&pool, &user_id, &key.id)
        .await
        .unwrap();
    finalize_limit_admission(&pool, &admission, 23)
        .await
        .unwrap();
    let finalized: (i64, Option<String>) =
        sqlx::query_as("SELECT total_tokens, finalized_at FROM limit_usage_events WHERE id = ?")
            .bind(&admission.usage_event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let usage_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_usage_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let inflight_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    let rate_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_rate_counters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(finalized.0, 23);
    assert!(finalized.1.is_some());
    assert_eq!((usage_count, inflight_count, rate_count), (1, 0, 2));
}

#[tokio::test]
async fn limit_hot_paths_scope_history_and_fail_closed_for_targeted_rows() {
    let pool = connect_and_migrate("sqlite://:memory:").await.unwrap();
    let (user_id, key) = seeded_limit_principal(&pool, "bounded-current@example.com").await;
    let (other_user_id, other_key) =
        seeded_limit_principal(&pool, "bounded-other@example.com").await;
    upsert_limit_policy(
        &pool,
        "system",
        "",
        &LimitPolicyPatch {
            request_quota: LimitPatchValue::Set(10_000),
            request_window_seconds: Some(60),
            token_quota: LimitPatchValue::Set(10_000),
            token_window_seconds: Some(60),
            rate_limit_requests: LimitPatchValue::Set(10_000),
            rate_limit_window_seconds: Some(60),
            concurrency_limit: LimitPatchValue::Set(10),
        },
    )
    .await
    .unwrap();

    let historical_base = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut events = QueryBuilder::new(
        "INSERT INTO limit_usage_events
         (id, user_id, api_key_id, request_count, total_tokens, created_at, finalized_at) ",
    );
    events.push_values(0..1_500, |mut row, index| {
        let created_at = (historical_base + Duration::seconds(index))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        row.push_bind(format!("unrelated-event-{index}"))
            .push_bind(&other_user_id)
            .push_bind(&other_key.id)
            .push_bind(1_i64)
            .push_bind(1_i64)
            .push_bind(created_at.clone())
            .push_bind(Some(created_at));
    });
    events.build().execute(&pool).await.unwrap();

    let mut counters = QueryBuilder::new(
        "INSERT INTO limit_rate_counters
         (scope, subject_id, window_started_at, request_count, updated_at) ",
    );
    counters.push_values(0..1_500, |mut row, index| {
        let timestamp = (historical_base + Duration::minutes(index))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        row.push_bind("api_key")
            .push_bind(&other_key.id)
            .push_bind(timestamp.clone())
            .push_bind(1_i64)
            .push_bind(timestamp);
    });
    counters.build().execute(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO limit_usage_events
         (id, user_id, api_key_id, request_count, total_tokens, created_at, finalized_at)
         VALUES ('old-current-event', ?, ?, 1, 1, '2020-01-01T00:00:00.000Z', ?)",
    )
    .bind(&user_id)
    .bind(&key.id)
    .bind("irrelevant-old-event-corruption")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO limit_rate_counters
         (scope, subject_id, window_started_at, request_count, updated_at)
         VALUES ('api_key', ?, '2020-01-01T00:00:00.000Z', 1, ?)",
    )
    .bind(&key.id)
    .bind("irrelevant-old-counter-corruption")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO limit_inflight_requests
         (id, user_id, api_key_id, started_at, expires_at)
         VALUES ('unrelated-inflight', ?, ?, ?, ?)",
    )
    .bind(&other_user_id)
    .bind(&other_key.id)
    .bind("irrelevant-inflight-corruption")
    .bind("irrelevant-inflight-corruption")
    .execute(&pool)
    .await
    .unwrap();

    let usage_plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {}",
        super::limits::USER_USAGE_WINDOW_SQL
    ))
    .bind(&user_id)
    .bind("2026-07-11T23:59:00.000Z")
    .fetch_all(&pool)
    .await
    .unwrap();
    let usage_details = usage_plan
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>();
    assert!(
        usage_details
            .iter()
            .any(|detail| detail.contains("idx_limit_usage_user_created")),
        "usage query plan was {usage_details:?}"
    );
    assert!(
        usage_details
            .iter()
            .all(|detail| !detail.contains("SCAN limit_usage_events"))
    );

    let rate_plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {}",
        super::limits::CURRENT_RATE_COUNTER_SQL
    ))
    .bind("api_key")
    .bind(&key.id)
    .bind("2026-07-12T00:00:00.000Z")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(rate_plan.iter().any(|row| {
        row.get::<String, _>("detail")
            .contains("sqlite_autoindex_limit_rate_counters_1")
    }));

    let inflight_plan = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {}",
        super::limits::USER_INFLIGHT_SQL
    ))
    .bind(&user_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(inflight_plan.iter().any(|row| {
        row.get::<String, _>("detail")
            .contains("idx_limit_inflight_user")
    }));
    let source = include_str!("limits.rs");
    assert!(!source.contains("validate_limit_timestamps"));
    assert!(!source.contains("UNION ALL SELECT created_at FROM limit_usage_events"));

    let now = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let clock: SharedClock = Arc::new(ManualClock::new(now));
    let admission =
        super::limits::admit_limited_request_with_clock(&pool, &user_id, &key.id, clock.clone())
            .await
            .unwrap();
    super::limits::finalize_limit_admission_with_clock(&pool, &admission, 3, clock.clone())
        .await
        .unwrap();

    sqlx::query("UPDATE limit_usage_events SET finalized_at = ? WHERE id = ?")
        .bind("targeted-event-corruption")
        .bind(&admission.usage_event_id)
        .execute(&pool)
        .await
        .unwrap();
    let error =
        super::limits::admit_limited_request_with_clock(&pool, &user_id, &key.id, clock.clone())
            .await
            .unwrap_err();
    let LimitAdmissionError::Storage(error) = error else {
        panic!("targeted corruption did not return storage integrity error");
    };
    assert!(is_data_integrity_error(&error));
    assert!(!error.to_string().contains("targeted-event-corruption"));

    sqlx::query("UPDATE limit_usage_events SET finalized_at = ? WHERE id = ?")
        .bind("2026-07-12T00:00:00.000Z")
        .bind(&admission.usage_event_id)
        .execute(&pool)
        .await
        .unwrap();
    let second =
        super::limits::admit_limited_request_with_clock(&pool, &user_id, &key.id, clock.clone())
            .await
            .unwrap();
    sqlx::query("UPDATE limit_usage_events SET created_at = ? WHERE id = ?")
        .bind("targeted-finalization-corruption")
        .bind(&second.usage_event_id)
        .execute(&pool)
        .await
        .unwrap();
    let error =
        super::limits::finalize_limit_admission_with_clock(&pool, &second, 7, clock.clone())
            .await
            .unwrap_err();
    assert!(is_data_integrity_error(&error));
    assert!(
        !error
            .to_string()
            .contains("targeted-finalization-corruption")
    );
    let unchanged: (i64, Option<String>) =
        sqlx::query_as("SELECT total_tokens, finalized_at FROM limit_usage_events WHERE id = ?")
            .bind(&second.usage_event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(unchanged, (0, None));

    sqlx::query("UPDATE limit_usage_events SET created_at = ? WHERE id = ?")
        .bind("2026-07-12T00:00:00.000Z")
        .bind(&second.usage_event_id)
        .execute(&pool)
        .await
        .unwrap();
    super::limits::finalize_limit_admission_with_clock(&pool, &second, 7, clock)
        .await
        .unwrap();
}

#[tokio::test]
async fn commit_failure_evicts_connection_and_allows_replacement_transaction() {
    let (_temp_dir, pool) = single_connection_file_pool("commit-failure.db").await;
    let actor_id = ensure_user(
        &pool,
        &CreateUser {
            email: "commit-failure-admin@example.com".into(),
            password: "password".into(),
            role: "admin".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("CREATE TEMP TABLE failed_commit_connection_marker (id INTEGER)")
        .execute(&pool)
        .await
        .unwrap();

    let error = with_immediate_transaction::<_, sqlx::Error, _>(&pool, move |conn| {
        Box::pin(async move {
            sqlx::query("PRAGMA defer_foreign_keys = ON")
                .execute(&mut *conn)
                .await?;
            sqlx::query(
                "INSERT INTO api_keys
                     (id, user_id, name, key_prefix, key_hash, status, created_at)
                     VALUES ('failed-commit-key', 'missing-user', 'failed commit',
                             'failed-prefix', 'failed-hash', 'active', ?)",
            )
            .bind(now_string())
            .execute(&mut *conn)
            .await?;
            Ok(())
        })
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("FOREIGN KEY constraint failed"));

    let connection_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_temp_master
             WHERE type = 'table' AND name = 'failed_commit_connection_marker'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let failed_key_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE id = 'failed-commit-key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(connection_marker_count, 0);
    assert_eq!(failed_key_count, 0);

    let recovered_id = "commit-failure-recovered-user".to_string();
    let recovered_id_for_query = recovered_id.clone();
    with_admin_audit::<_, sqlx::Error, _>(&pool, move |conn| {
        Box::pin(async move {
            let now = now_string();
            sqlx::query(
                "INSERT INTO users
                     (id, email, password_hash, role, status, created_at, updated_at)
                     VALUES (?, 'commit-recovered@example.com', 'unused',
                             'user', 'active', ?, ?)",
            )
            .bind(&recovered_id)
            .bind(&now)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            Ok((
                (),
                AdminAuditInsert {
                    actor_user_id: actor_id,
                    actor_email: "commit-failure-admin@example.com".into(),
                    action: "commit_failure_recovered",
                    resource_type: "user",
                    resource_id: Some(recovered_id),
                    status: "success",
                    metadata_json: Some("{}".into()),
                },
            ))
        })
    })
    .await
    .unwrap();
    let recovered_user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ?")
        .bind(&recovered_id_for_query)
        .fetch_one(&pool)
        .await
        .unwrap();
    let recovered_audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_logs
             WHERE action = 'commit_failure_recovered' AND resource_id = ?",
    )
    .bind(&recovered_id_for_query)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(recovered_user_count, 1);
    assert_eq!(recovered_audit_count, 1);
}

#[tokio::test]
async fn health_transition_timestamps_refresh_on_repeated_transitions() {
    let pool = connect_and_migrate("sqlite://:memory:").await.unwrap();
    let upstream = create_upstream(
        &pool,
        "test-secret",
        1,
        &UpsertUpstream {
            name: "flaky".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "sk-flaky".into(),
            enabled: Some(true),
            priority: Some(1),
            weight: Some(1),
            timeout_ms: TimeoutPatchValue::Default,
            max_retries: None,
            health_check_path: None,
        },
    )
    .await
    .unwrap();

    let transitions = [
        ("down", Some("first_down"), "2026-07-12T00:00:01.000Z"),
        ("healthy", None, "2026-07-12T00:00:02.000Z"),
        ("down", Some("second_down"), "2026-07-12T00:00:03.000Z"),
        ("healthy", None, "2026-07-12T00:00:04.000Z"),
        (
            "degraded",
            Some("first_degraded"),
            "2026-07-12T00:00:05.000Z",
        ),
        ("healthy", None, "2026-07-12T00:00:06.000Z"),
        (
            "degraded",
            Some("second_degraded"),
            "2026-07-12T00:00:07.000Z",
        ),
    ];
    for (status, error, now) in transitions {
        record_upstream_health_at(&pool, &upstream.id, status, error, now)
            .await
            .unwrap();
    }

    let current = get_upstream(&pool, &upstream.id).await.unwrap().unwrap();
    assert_eq!(
        current.health_status_changed_at.as_deref(),
        Some(transitions[6].2)
    );
    assert_eq!(current.last_down_at.as_deref(), Some(transitions[2].2));
    assert_eq!(current.last_degraded_at.as_deref(), Some(transitions[6].2));
    assert!(current.recent_error_samples.contains("first_down"));
    assert!(current.recent_error_samples.contains("second_down"));
    assert!(current.recent_error_samples.contains("second_degraded"));
}

#[tokio::test]
async fn second_limit_settlement_statement_failure_rolls_back_usage_finalization() {
    let pool = connect_and_migrate("sqlite://:memory:").await.unwrap();
    let user_id = ensure_user(
        &pool,
        &CreateUser {
            email: "settlement-failure@example.com".into(),
            password: "password".into(),
            role: "user".into(),
            display_name: None,
        },
    )
    .await
    .unwrap();
    let (api_key, _) = create_api_key(
        &pool,
        "test-secret",
        &user_id,
        &CreateApiKey {
            name: "settlement-failure".into(),
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let admission = admit_limited_request(&pool, &user_id, &api_key.id)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_inflight_cleanup
         BEFORE DELETE ON limit_inflight_requests
         BEGIN
             SELECT RAISE(FAIL, 'injected inflight cleanup failure');
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = finalize_limit_admission(&pool, &admission, 37)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected inflight cleanup failure")
    );
    let usage: (i64, Option<String>) =
        sqlx::query_as("SELECT total_tokens, finalized_at FROM limit_usage_events WHERE id = ?")
            .bind(&admission.usage_event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let inflight_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests WHERE id = ?")
            .bind(&admission.inflight_request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(usage, (0, None));
    assert_eq!(inflight_count, 1);

    sqlx::query("DROP TRIGGER fail_inflight_cleanup")
        .execute(&pool)
        .await
        .unwrap();
    finalize_limit_admission(&pool, &admission, 37)
        .await
        .unwrap();
    let usage: (i64, Option<String>) =
        sqlx::query_as("SELECT total_tokens, finalized_at FROM limit_usage_events WHERE id = ?")
            .bind(&admission.usage_event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(usage.0, 37);
    assert!(usage.1.is_some());
    let inflight_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(inflight_count, 0);
}

#[tokio::test]
async fn limit_admission_reads_clock_after_lock_wait_crosses_rate_window() {
    let (_temp_dir, pool) = single_connection_file_pool("clocked-rate-window.db").await;
    let (user_id, key) = seeded_limit_principal(&pool, "clocked-rate@example.com").await;
    upsert_limit_policy(
        &pool,
        "system",
        "",
        &LimitPolicyPatch {
            rate_limit_requests: LimitPatchValue::Set(1),
            rate_limit_window_seconds: Some(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let initial = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let manual = ManualClock::new(initial);
    let clock: SharedClock = Arc::new(manual.clone());
    let first =
        super::limits::admit_limited_request_with_clock(&pool, &user_id, &key.id, clock.clone())
            .await
            .unwrap();
    finalize_limit_admission(&pool, &first, 0).await.unwrap();

    let (blocker, release) = hold_only_connection(&pool).await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let waiting_pool = pool.clone();
    let waiting_user = user_id.clone();
    let waiting_key = key.id.clone();
    let admission = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        super::limits::admit_limited_request_with_clock(
            &waiting_pool,
            &waiting_user,
            &waiting_key,
            clock,
        )
        .await
    });
    started_rx.await.unwrap();
    manual.advance(Duration::seconds(61));
    release.notify_one();
    blocker.await.unwrap();
    let second = admission.await.unwrap().unwrap();

    let (created_at, started_at, expires_at): (String, String, String) = sqlx::query_as(
        "SELECT usage.created_at, inflight.started_at, inflight.expires_at
         FROM limit_usage_events usage
         JOIN limit_inflight_requests inflight ON inflight.id = ?
         WHERE usage.id = ?",
    )
    .bind(&second.inflight_request_id)
    .bind(&second.usage_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(created_at, "2026-07-12T00:01:01.000Z");
    assert_eq!(started_at, created_at);
    assert_eq!(expires_at, "2026-07-12T06:01:01.000Z");
}

#[tokio::test]
async fn stale_inflight_cleanup_reads_clock_after_lock_wait() {
    let (_temp_dir, pool) = single_connection_file_pool("clocked-stale-inflight.db").await;
    let (user_id, key) = seeded_limit_principal(&pool, "clocked-stale@example.com").await;
    upsert_limit_policy(
        &pool,
        "system",
        "",
        &LimitPolicyPatch {
            concurrency_limit: LimitPatchValue::Set(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let initial = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let manual = ManualClock::new(initial);
    let clock: SharedClock = Arc::new(manual.clone());
    let stale =
        super::limits::admit_limited_request_with_clock(&pool, &user_id, &key.id, clock.clone())
            .await
            .unwrap();

    let (blocker, release) = hold_only_connection(&pool).await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let waiting_pool = pool.clone();
    let waiting_user = user_id.clone();
    let waiting_key = key.id.clone();
    let admission = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        super::limits::admit_limited_request_with_clock(
            &waiting_pool,
            &waiting_user,
            &waiting_key,
            clock,
        )
        .await
    });
    started_rx.await.unwrap();
    manual.advance(Duration::hours(7));
    release.notify_one();
    blocker.await.unwrap();
    let current = admission.await.unwrap().unwrap();

    let stale_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_inflight_requests WHERE id = ?")
            .bind(&stale.inflight_request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let current_started_at: String =
        sqlx::query_scalar("SELECT started_at FROM limit_inflight_requests WHERE id = ?")
            .bind(&current.inflight_request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stale_count, 0);
    assert_eq!(current_started_at, "2026-07-12T07:00:00.000Z");
}

#[tokio::test]
async fn retention_reads_clock_after_lock_wait_crosses_cutoff() {
    let (_temp_dir, pool) = single_connection_file_pool("clocked-retention.db").await;
    let (user_id, key) = seeded_limit_principal(&pool, "clocked-retention@example.com").await;
    insert_request_log(
        &pool,
        RequestLogInsert {
            request_id: "crossing-retention-cutoff".into(),
            user_id,
            api_key_id: key.id,
            model_id: None,
            upstream_id: None,
            method: "POST".into(),
            path: "/responses".into(),
            status_code: Some(200),
            error_code: None,
            stream: false,
            usage: crate::usage::UsageSnapshot::default(),
            input_chars: 0,
            output_chars: 0,
            latency_ms: 1,
            started_at: "2026-07-11T12:00:00.000Z".into(),
            finished_at: "2026-07-11T12:00:00.000Z".into(),
            client_ip_hash: None,
            user_agent: None,
            client_metadata_sanitized: None,
            route_strategy: None,
            route_decision_json: None,
        },
    )
    .await
    .unwrap();
    let initial = DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let manual = ManualClock::new(initial);
    let clock: SharedClock = Arc::new(manual.clone());
    let policy = RetentionPolicy {
        request_log_retention_days: 1,
        daily_usage_retention_days: 0,
    };

    let (blocker, release) = hold_only_connection(&pool).await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let waiting_pool = pool.clone();
    let retention = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        super::request_logs::apply_retention_with_clock(&waiting_pool, &policy, clock).await
    });
    started_rx.await.unwrap();
    manual.advance(Duration::hours(13));
    release.notify_one();
    blocker.await.unwrap();
    let result = retention.await.unwrap().unwrap();

    assert_eq!(result.request_logs_deleted, 1);
    assert_eq!(
        result.request_log_cutoff.as_deref(),
        Some("2026-07-11T13:00:00.000Z")
    );
    assert!(list_request_logs(&pool, None).await.unwrap().is_empty());
}
