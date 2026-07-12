use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};

use crate::{
    auth,
    clock::{SharedClock, system_clock},
    usage::UsageSnapshot,
};

use super::db::{bool_to_i64, now_string, push_where, with_immediate_transaction};

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct RequestLogRow {
    pub id: String,
    pub request_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: Option<i64>,
    pub error_code: Option<String>,
    pub stream: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub usage_source: String,
    pub input_chars: i64,
    pub output_chars: i64,
    pub latency_ms: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub client_ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub upstream_response_id: Option<String>,
    pub upstream_status: Option<String>,
    pub client_metadata_sanitized: Option<String>,
    pub route_strategy: Option<String>,
    pub route_decision_json: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestLogFilters {
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub status_code: Option<i64>,
    pub error_only: bool,
    pub started_at_from: Option<String>,
    pub started_at_to: Option<String>,
    pub latency_min_ms: Option<i64>,
    pub latency_max_ms: Option<i64>,
    pub limit: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct RetentionPolicy {
    pub request_log_retention_days: i64,
    pub daily_usage_retention_days: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetentionResult {
    pub request_logs_deleted: u64,
    pub daily_usage_deleted: u64,
    pub request_log_cutoff: Option<String>,
    pub daily_usage_cutoff: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RequestLogInsert {
    pub request_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: Option<i64>,
    pub error_code: Option<String>,
    pub stream: bool,
    pub usage: UsageSnapshot,
    pub input_chars: i64,
    pub output_chars: i64,
    pub latency_ms: i64,
    pub started_at: String,
    pub finished_at: String,
    pub client_ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub client_metadata_sanitized: Option<String>,
    pub route_strategy: Option<String>,
    pub route_decision_json: Option<String>,
}
pub async fn list_request_logs(
    pool: &SqlitePool,
    user_id: Option<&str>,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let filters = RequestLogFilters {
        user_id: user_id.map(str::to_string),
        limit: Some(if user_id.is_some() { 200 } else { 500 }),
        ..Default::default()
    };
    list_request_logs_filtered(pool, &filters).await
}

pub async fn list_request_logs_filtered(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new("SELECT * FROM request_logs");
    push_request_log_filter_where(&mut query, filters);
    query.push(" ORDER BY started_at DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(500).clamp(1, 1000));
    query.build_query_as().fetch_all(pool).await
}

pub(super) fn push_request_log_filter_where<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    filters: &'a RequestLogFilters,
) {
    let mut has_where = false;
    if let Some(user_id) = &filters.user_id {
        push_where(query, &mut has_where);
        query.push("user_id = ").push_bind(user_id);
    }
    if let Some(api_key_id) = &filters.api_key_id {
        push_where(query, &mut has_where);
        query.push("api_key_id = ").push_bind(api_key_id);
    }
    if let Some(model_id) = &filters.model_id {
        push_where(query, &mut has_where);
        query.push("model_id = ").push_bind(model_id);
    }
    if let Some(upstream_id) = &filters.upstream_id {
        push_where(query, &mut has_where);
        query.push("upstream_id = ").push_bind(upstream_id);
    }
    if let Some(status_code) = filters.status_code {
        push_where(query, &mut has_where);
        query.push("status_code = ").push_bind(status_code);
    }
    if filters.error_only {
        push_where(query, &mut has_where);
        query.push("COALESCE(status_code, 500) >= 400");
    }
    if let Some(started_at_from) = &filters.started_at_from {
        push_where(query, &mut has_where);
        query.push("started_at >= ").push_bind(started_at_from);
    }
    if let Some(started_at_to) = &filters.started_at_to {
        push_where(query, &mut has_where);
        query.push("started_at <= ").push_bind(started_at_to);
    }
    if let Some(latency_min_ms) = filters.latency_min_ms {
        push_where(query, &mut has_where);
        query.push("latency_ms >= ").push_bind(latency_min_ms);
    }
    if let Some(latency_max_ms) = filters.latency_max_ms {
        push_where(query, &mut has_where);
        query.push("latency_ms <= ").push_bind(latency_max_ms);
    }
}

pub(super) fn request_log_filters_empty(filters: &RequestLogFilters) -> bool {
    filters.user_id.is_none()
        && filters.api_key_id.is_none()
        && filters.model_id.is_none()
        && filters.upstream_id.is_none()
        && filters.status_code.is_none()
        && !filters.error_only
        && filters.started_at_from.is_none()
        && filters.started_at_to.is_none()
        && filters.latency_min_ms.is_none()
        && filters.latency_max_ms.is_none()
}

pub async fn apply_retention(
    pool: &SqlitePool,
    policy: &RetentionPolicy,
) -> sqlx::Result<RetentionResult> {
    apply_retention_with_clock(pool, policy, system_clock()).await
}

pub(crate) async fn apply_retention_with_clock(
    pool: &SqlitePool,
    policy: &RetentionPolicy,
    clock: SharedClock,
) -> sqlx::Result<RetentionResult> {
    let policy = policy.clone();
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move { apply_retention_conn(conn, &policy, clock.now()).await })
    })
    .await
}

pub async fn apply_retention_at(
    pool: &SqlitePool,
    policy: &RetentionPolicy,
    now: DateTime<Utc>,
) -> sqlx::Result<RetentionResult> {
    let policy = policy.clone();
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move { apply_retention_conn(conn, &policy, now).await })
    })
    .await
}

pub async fn apply_retention_conn(
    conn: &mut sqlx::SqliteConnection,
    policy: &RetentionPolicy,
    now: DateTime<Utc>,
) -> sqlx::Result<RetentionResult> {
    let request_log_cutoff = (policy.request_log_retention_days > 0).then(|| {
        (now - Duration::days(policy.request_log_retention_days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    });
    let daily_usage_cutoff = (policy.daily_usage_retention_days > 0).then(|| {
        (now - Duration::days(policy.daily_usage_retention_days))
            .date_naive()
            .to_string()
    });

    let request_logs_deleted = if let Some(cutoff) = &request_log_cutoff {
        sqlx::query("DELETE FROM request_logs WHERE started_at < ?")
            .bind(cutoff)
            .execute(&mut *conn)
            .await?
            .rows_affected()
    } else {
        0
    };
    let daily_usage_deleted = if let Some(cutoff) = &daily_usage_cutoff {
        sqlx::query("DELETE FROM daily_usage WHERE date < ?")
            .bind(cutoff)
            .execute(&mut *conn)
            .await?
            .rows_affected()
    } else {
        0
    };

    Ok(RetentionResult {
        request_logs_deleted,
        daily_usage_deleted,
        request_log_cutoff,
        daily_usage_cutoff,
    })
}

pub async fn insert_request_log(pool: &SqlitePool, log: RequestLogInsert) -> sqlx::Result<()> {
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move {
            let id = auth::new_id();
            sqlx::query(
                "INSERT INTO request_logs
                 (id, request_id, user_id, api_key_id, model_id, upstream_id, method, path, status_code, error_code, stream,
                  prompt_tokens, completion_tokens, total_tokens, usage_source, input_chars, output_chars, latency_ms,
                  started_at, finished_at, client_ip_hash, user_agent, upstream_response_id, upstream_status, client_metadata_sanitized,
                  route_strategy, route_decision_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&log.request_id)
            .bind(&log.user_id)
            .bind(&log.api_key_id)
            .bind(&log.model_id)
            .bind(&log.upstream_id)
            .bind(&log.method)
            .bind(&log.path)
            .bind(log.status_code)
            .bind(&log.error_code)
            .bind(bool_to_i64(log.stream))
            .bind(log.usage.prompt_tokens)
            .bind(log.usage.completion_tokens)
            .bind(log.usage.total_tokens)
            .bind(log.usage.source.as_str())
            .bind(log.input_chars)
            .bind(log.output_chars)
            .bind(log.latency_ms)
            .bind(&log.started_at)
            .bind(&log.finished_at)
            .bind(&log.client_ip_hash)
            .bind(&log.user_agent)
            .bind(&log.usage.upstream_response_id)
            .bind(&log.usage.upstream_status)
            .bind(&log.client_metadata_sanitized)
            .bind(&log.route_strategy)
            .bind(&log.route_decision_json)
            .execute(&mut *conn)
            .await?;

            upsert_daily_usage(conn, &log).await
        })
    })
    .await
}

async fn upsert_daily_usage(
    conn: &mut sqlx::SqliteConnection,
    log: &RequestLogInsert,
) -> sqlx::Result<()> {
    let day = log.started_at.get(0..10).unwrap_or("unknown");
    let error_count = i64::from(log.status_code.unwrap_or(500) >= 400);
    let stream_count = i64::from(log.stream);
    sqlx::query(
        "INSERT INTO daily_usage
         (id, date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count,
          prompt_tokens, completion_tokens, total_tokens, latency_ms_sum, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(date, user_id, api_key_id, model_id, upstream_id) DO UPDATE SET
            request_count = request_count + 1,
            error_count = error_count + excluded.error_count,
            stream_count = stream_count + excluded.stream_count,
            prompt_tokens = prompt_tokens + excluded.prompt_tokens,
            completion_tokens = completion_tokens + excluded.completion_tokens,
            total_tokens = total_tokens + excluded.total_tokens,
            latency_ms_sum = latency_ms_sum + excluded.latency_ms_sum,
            updated_at = excluded.updated_at",
    )
    .bind(auth::new_id())
    .bind(day)
    .bind(&log.user_id)
    .bind(&log.api_key_id)
    .bind(&log.model_id)
    .bind(&log.upstream_id)
    .bind(error_count)
    .bind(stream_count)
    .bind(log.usage.prompt_tokens)
    .bind(log.usage.completion_tokens)
    .bind(log.usage.total_tokens)
    .bind(log.latency_ms)
    .bind(now_string())
    .bind(now_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}
