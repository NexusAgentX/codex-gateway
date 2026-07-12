use std::{str::FromStr, sync::Arc};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use super::*;
use super::{db::with_immediate_transaction, limits::admit_limited_request_in_tx};

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
