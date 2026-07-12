use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};

use super::{
    api_keys::ApiKeySummary,
    db::{now_string, push_where},
    limits::LimitSubjectState,
    request_logs::{
        RequestLogFilters, RequestLogRow, push_request_log_filter_where, request_log_filters_empty,
    },
};

type GatewayMetricsTotalsRow = (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct DailyUsageRow {
    pub date: String,
    pub user_id: String,
    pub api_key_id: String,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub stream_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
}

#[derive(Clone, Debug, Default)]
pub struct DailyUsageFilters {
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub model_id: Option<String>,
    pub upstream_id: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageTotals {
    pub request_count: i64,
    pub error_count: i64,
    pub stream_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
    pub error_rate: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct ErrorSummaryRow {
    pub error_code: String,
    pub status_code: Option<i64>,
    pub count: i64,
    pub last_seen_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageSummary {
    pub totals: UsageTotals,
    pub errors: Vec<ErrorSummaryRow>,
    pub recent_failures: Vec<RequestLogRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiKeyUsageSummary {
    pub api_key: ApiKeySummary,
    pub usage: UsageSummary,
    pub limits: Option<LimitSubjectState>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GatewayMetrics {
    pub generated_at: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency: LatencyMetrics,
    pub token_usage: TokenUsageMetrics,
    pub upstream_health: Vec<UpstreamHealthMetrics>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LatencyMetrics {
    pub sum_ms: i64,
    pub avg_ms: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenUsageMetrics {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct UpstreamHealthMetrics {
    pub upstream_id: String,
    pub name: String,
    pub enabled: i64,
    pub last_health_status: String,
    pub last_health_checked_at: Option<String>,
    pub last_degraded_at: Option<String>,
    pub last_down_at: Option<String>,
    pub recent_error_samples: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency_ms_sum: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsSnapshot {
    pub generated_at: String,
    pub requests_24h: Vec<AnalyticsRequestBucket>,
    pub token_usage_7d: Vec<AnalyticsTokenBucket>,
    pub model_share: Vec<AnalyticsDimensionShare>,
    pub upstream_error_rate: Vec<AnalyticsUpstreamErrorRate>,
    pub user_error_rate: Vec<AnalyticsUserErrorRate>,
    pub latency_trend: Vec<AnalyticsLatencyTrendBucket>,
    pub latency_buckets: Vec<AnalyticsLatencyBucket>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct AnalyticsRequestBucket {
    pub bucket: String,
    pub request_count: i64,
    pub error_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct AnalyticsTokenBucket {
    pub date: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsDimensionShare {
    pub id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
    pub share: f64,
}

#[derive(Clone, Debug, FromRow)]
struct AnalyticsDimensionRow {
    pub id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub total_tokens: i64,
    pub latency_ms_sum: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsUpstreamErrorRate {
    pub upstream_id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub error_rate: f64,
    pub avg_latency_ms: Option<f64>,
}

#[derive(Clone, Debug, FromRow)]
struct AnalyticsUpstreamErrorRateRow {
    pub upstream_id: Option<String>,
    pub request_count: i64,
    pub error_count: i64,
    pub latency_ms_sum: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsUserErrorRate {
    pub user_id: String,
    pub request_count: i64,
    pub error_count: i64,
    pub error_rate: f64,
    pub avg_latency_ms: Option<f64>,
}

#[derive(Clone, Debug, FromRow)]
struct AnalyticsUserErrorRateRow {
    pub user_id: String,
    pub request_count: i64,
    pub error_count: i64,
    pub latency_ms_sum: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct AnalyticsLatencyTrendBucket {
    pub bucket: String,
    pub request_count: i64,
    pub error_count: i64,
    pub avg_latency_ms: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalyticsLatencyBucket {
    pub label: String,
    pub min_ms: i64,
    pub max_ms: Option<i64>,
    pub request_count: i64,
    pub error_count: i64,
}

#[derive(Clone, Debug, FromRow)]
struct AnalyticsLatencyBucketRow {
    pub sort_order: i64,
    pub label: String,
    pub min_ms: i64,
    pub max_ms: Option<i64>,
    pub request_count: i64,
    pub error_count: i64,
}

pub async fn list_daily_usage(
    pool: &SqlitePool,
    user_id: Option<&str>,
) -> sqlx::Result<Vec<DailyUsageRow>> {
    let filters = DailyUsageFilters {
        user_id: user_id.map(str::to_string),
        limit: Some(if user_id.is_some() { 90 } else { 500 }),
        ..Default::default()
    };
    list_daily_usage_filtered(pool, &filters).await
}

pub async fn list_daily_usage_filtered(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<Vec<DailyUsageRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT date, user_id, api_key_id, model_id, upstream_id, request_count, error_count, stream_count,
                prompt_tokens, completion_tokens, total_tokens, latency_ms_sum
         FROM daily_usage",
    );
    push_daily_usage_filters(&mut query, filters);
    query.push(" ORDER BY date DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(500).clamp(1, 1000));
    query.build_query_as().fetch_all(pool).await
}

pub async fn usage_summary(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<UsageSummary> {
    let mut totals_query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT
            COALESCE(SUM(request_count), 0),
            COALESCE(SUM(error_count), 0),
            COALESCE(SUM(stream_count), 0),
            COALESCE(SUM(prompt_tokens), 0),
            COALESCE(SUM(completion_tokens), 0),
            COALESCE(SUM(total_tokens), 0),
            COALESCE(SUM(latency_ms_sum), 0)
         FROM daily_usage",
    );
    push_daily_usage_filters(&mut totals_query, filters);
    let totals: (i64, i64, i64, i64, i64, i64, i64) =
        totals_query.build_query_as().fetch_one(pool).await?;
    let error_rate = if totals.0 > 0 {
        totals.1 as f64 / totals.0 as f64
    } else {
        0.0
    };
    let request_filters = request_filters_from_usage(filters, Some(12));
    Ok(UsageSummary {
        totals: UsageTotals {
            request_count: totals.0,
            error_count: totals.1,
            stream_count: totals.2,
            prompt_tokens: totals.3,
            completion_tokens: totals.4,
            total_tokens: totals.5,
            latency_ms_sum: totals.6,
            error_rate,
        },
        errors: error_summary(pool, filters).await?,
        recent_failures: list_recent_failures(pool, &request_filters).await?,
    })
}

pub async fn api_key_usage_summary(
    pool: &SqlitePool,
    api_key: ApiKeySummary,
    include_limits: bool,
) -> sqlx::Result<ApiKeyUsageSummary> {
    api_key_usage_summary_at(pool, api_key, include_limits, Utc::now()).await
}

pub(crate) async fn api_key_usage_summary_at(
    pool: &SqlitePool,
    api_key: ApiKeySummary,
    include_limits: bool,
    now: chrono::DateTime<Utc>,
) -> sqlx::Result<ApiKeyUsageSummary> {
    let filters = DailyUsageFilters {
        user_id: Some(api_key.user_id.clone()),
        api_key_id: Some(api_key.id.clone()),
        limit: Some(90),
        ..DailyUsageFilters::default()
    };
    let limits = if include_limits {
        super::limits::user_limit_state_at(pool, &api_key.user_id, Some(&api_key.id), now)
            .await?
            .current_key
    } else {
        None
    };
    Ok(ApiKeyUsageSummary {
        api_key,
        usage: usage_summary(pool, &filters).await?,
        limits,
    })
}

fn push_daily_usage_filters<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    filters: &'a DailyUsageFilters,
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
    if let Some(date_from) = &filters.date_from {
        push_where(query, &mut has_where);
        query.push("date >= ").push_bind(date_from);
    }
    if let Some(date_to) = &filters.date_to {
        push_where(query, &mut has_where);
        query.push("date <= ").push_bind(date_to);
    }
}

async fn list_recent_failures(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<RequestLogRow>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new("SELECT * FROM request_logs");
    push_request_log_filter_where(&mut query, filters);
    query.push(if request_log_filters_empty(filters) {
        " WHERE "
    } else {
        " AND "
    });
    query.push("COALESCE(status_code, 500) >= 400 ORDER BY started_at DESC LIMIT ");
    query.push_bind(filters.limit.unwrap_or(12).clamp(1, 100));
    query.build_query_as().fetch_all(pool).await
}

async fn error_summary(
    pool: &SqlitePool,
    filters: &DailyUsageFilters,
) -> sqlx::Result<Vec<ErrorSummaryRow>> {
    let request_filters = request_filters_from_usage(filters, None);
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT COALESCE(error_code, 'http_' || COALESCE(status_code, 500)) AS error_code,
                status_code,
                COUNT(*) AS count,
                MAX(started_at) AS last_seen_at
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, &request_filters);
    query.push(if request_log_filters_empty(&request_filters) {
        " WHERE "
    } else {
        " AND "
    });
    query.push(
        "COALESCE(status_code, 500) >= 400
         GROUP BY COALESCE(error_code, 'http_' || COALESCE(status_code, 500)), status_code
         ORDER BY count DESC, last_seen_at DESC
         LIMIT 20",
    );
    query.build_query_as().fetch_all(pool).await
}

fn request_filters_from_usage(
    filters: &DailyUsageFilters,
    limit: Option<i64>,
) -> RequestLogFilters {
    RequestLogFilters {
        user_id: filters.user_id.clone(),
        api_key_id: filters.api_key_id.clone(),
        model_id: filters.model_id.clone(),
        upstream_id: filters.upstream_id.clone(),
        status_code: None,
        error_only: false,
        started_at_from: filters
            .date_from
            .as_ref()
            .map(|date| format!("{date}T00:00:00.000Z")),
        started_at_to: filters
            .date_to
            .as_ref()
            .map(|date| format!("{date}T23:59:59.999Z")),
        latency_min_ms: None,
        latency_max_ms: None,
        limit,
    }
}

pub async fn gateway_metrics(pool: &SqlitePool) -> sqlx::Result<GatewayMetrics> {
    let totals: GatewayMetricsTotalsRow = sqlx::query_as(
        "SELECT
                SUM(request_count),
                SUM(error_count),
                SUM(latency_ms_sum),
                SUM(prompt_tokens),
                SUM(completion_tokens),
                SUM(total_tokens)
             FROM daily_usage",
    )
    .fetch_one(pool)
    .await?;
    let request_count = totals.0.unwrap_or_default();
    let latency_sum = totals.2.unwrap_or_default();
    let upstream_health = sqlx::query_as(
        "SELECT
            upstreams.id AS upstream_id,
            upstreams.name AS name,
            upstreams.enabled AS enabled,
            upstreams.last_health_status AS last_health_status,
            upstreams.last_health_checked_at AS last_health_checked_at,
            upstreams.last_degraded_at AS last_degraded_at,
            upstreams.last_down_at AS last_down_at,
            upstreams.recent_error_samples AS recent_error_samples,
            COALESCE(SUM(daily_usage.request_count), 0) AS request_count,
            COALESCE(SUM(daily_usage.error_count), 0) AS error_count,
            COALESCE(SUM(daily_usage.latency_ms_sum), 0) AS latency_ms_sum,
            COALESCE(SUM(daily_usage.total_tokens), 0) AS total_tokens
         FROM upstreams
         LEFT JOIN daily_usage ON daily_usage.upstream_id = upstreams.id
         GROUP BY upstreams.id
         ORDER BY upstreams.enabled DESC,
                  CASE upstreams.last_health_status
                    WHEN 'down' THEN 0
                    WHEN 'degraded' THEN 1
                    WHEN 'unknown' THEN 2
                    ELSE 3
                  END,
                  upstreams.priority,
                  upstreams.name",
    )
    .fetch_all(pool)
    .await?;

    Ok(GatewayMetrics {
        generated_at: now_string(),
        request_count,
        error_count: totals.1.unwrap_or_default(),
        latency: LatencyMetrics {
            sum_ms: latency_sum,
            avg_ms: (request_count > 0).then_some(latency_sum as f64 / request_count as f64),
        },
        token_usage: TokenUsageMetrics {
            prompt_tokens: totals.3.unwrap_or_default(),
            completion_tokens: totals.4.unwrap_or_default(),
            total_tokens: totals.5.unwrap_or_default(),
        },
        upstream_health,
    })
}

pub async fn analytics_snapshot(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<AnalyticsSnapshot> {
    Ok(AnalyticsSnapshot {
        generated_at: now_string(),
        requests_24h: analytics_requests_24h(pool, filters).await?,
        token_usage_7d: analytics_token_usage_7d(pool, filters).await?,
        model_share: analytics_model_share(pool, filters).await?,
        upstream_error_rate: analytics_upstream_error_rate(pool, filters).await?,
        user_error_rate: analytics_user_error_rate(pool, filters).await?,
        latency_trend: analytics_latency_trend(pool, filters).await?,
        latency_buckets: analytics_latency_buckets(pool, filters).await?,
    })
}

async fn analytics_requests_24h(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsRequestBucket>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT strftime('%Y-%m-%dT%H:00:00Z', started_at) AS bucket,
                COUNT(*) AS request_count,
                SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count
         FROM request_logs",
    );
    let mut windowed = filters.clone();
    if windowed.started_at_from.is_none() {
        windowed.started_at_from = Some(
            (Utc::now() - Duration::hours(24)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    push_request_log_filter_where(&mut query, &windowed);
    query.push(
        " GROUP BY strftime('%Y-%m-%dT%H:00:00Z', started_at)
          ORDER BY bucket",
    );
    query.build_query_as().fetch_all(pool).await
}

async fn analytics_token_usage_7d(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsTokenBucket>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT substr(started_at, 1, 10) AS date,
                COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COUNT(*) AS request_count
         FROM request_logs",
    );
    let mut windowed = filters.clone();
    if windowed.started_at_from.is_none() {
        windowed.started_at_from = Some(
            (Utc::now() - Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    push_request_log_filter_where(&mut query, &windowed);
    query.push(" GROUP BY substr(started_at, 1, 10) ORDER BY date");
    query.build_query_as().fetch_all(pool).await
}

async fn analytics_model_share(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsDimensionShare>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT model_id AS id,
                COUNT(*) AS request_count,
                SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(SUM(latency_ms), 0) AS latency_ms_sum
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, filters);
    query.push(
        " GROUP BY model_id
          ORDER BY request_count DESC, id
          LIMIT 20",
    );
    let rows: Vec<AnalyticsDimensionRow> = query.build_query_as().fetch_all(pool).await?;
    let total_requests: i64 = rows.iter().map(|row| row.request_count).sum();
    Ok(rows
        .into_iter()
        .map(|row| AnalyticsDimensionShare {
            id: row.id,
            request_count: row.request_count,
            error_count: row.error_count,
            total_tokens: row.total_tokens,
            latency_ms_sum: row.latency_ms_sum,
            share: if total_requests > 0 {
                row.request_count as f64 / total_requests as f64
            } else {
                0.0
            },
        })
        .collect())
}

async fn analytics_upstream_error_rate(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsUpstreamErrorRate>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT upstream_id,
                COUNT(*) AS request_count,
                SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count,
                COALESCE(SUM(latency_ms), 0) AS latency_ms_sum
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, filters);
    query.push(
        " GROUP BY upstream_id
          ORDER BY error_count DESC, request_count DESC, upstream_id
          LIMIT 20",
    );
    let rows: Vec<AnalyticsUpstreamErrorRateRow> = query.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| AnalyticsUpstreamErrorRate {
            upstream_id: row.upstream_id,
            request_count: row.request_count,
            error_count: row.error_count,
            error_rate: if row.request_count > 0 {
                row.error_count as f64 / row.request_count as f64
            } else {
                0.0
            },
            avg_latency_ms: (row.request_count > 0)
                .then_some(row.latency_ms_sum as f64 / row.request_count as f64),
        })
        .collect())
}

async fn analytics_user_error_rate(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsUserErrorRate>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT user_id,
                COUNT(*) AS request_count,
                SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count,
                COALESCE(SUM(latency_ms), 0) AS latency_ms_sum
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, filters);
    query.push(
        " GROUP BY user_id
          ORDER BY error_count DESC, request_count DESC, user_id
          LIMIT 20",
    );
    let rows: Vec<AnalyticsUserErrorRateRow> = query.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| AnalyticsUserErrorRate {
            user_id: row.user_id,
            request_count: row.request_count,
            error_count: row.error_count,
            error_rate: if row.request_count > 0 {
                row.error_count as f64 / row.request_count as f64
            } else {
                0.0
            },
            avg_latency_ms: (row.request_count > 0)
                .then_some(row.latency_ms_sum as f64 / row.request_count as f64),
        })
        .collect())
}

async fn analytics_latency_trend(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsLatencyTrendBucket>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT strftime('%Y-%m-%dT%H:00:00Z', started_at) AS bucket,
                COUNT(*) AS request_count,
                SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count,
                AVG(latency_ms) AS avg_latency_ms
         FROM request_logs",
    );
    let mut windowed = filters.clone();
    if windowed.started_at_from.is_none() {
        windowed.started_at_from = Some(
            (Utc::now() - Duration::hours(24)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    push_request_log_filter_where(&mut query, &windowed);
    query.push(
        " GROUP BY strftime('%Y-%m-%dT%H:00:00Z', started_at)
          ORDER BY bucket",
    );
    query.build_query_as().fetch_all(pool).await
}

async fn analytics_latency_buckets(
    pool: &SqlitePool,
    filters: &RequestLogFilters,
) -> sqlx::Result<Vec<AnalyticsLatencyBucket>> {
    let mut query: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
        "SELECT
            CASE
              WHEN latency_ms < 250 THEN 0
              WHEN latency_ms < 1000 THEN 1
              WHEN latency_ms < 3000 THEN 2
              WHEN latency_ms < 10000 THEN 3
              ELSE 4
            END AS sort_order,
            CASE
              WHEN latency_ms < 250 THEN '<250ms'
              WHEN latency_ms < 1000 THEN '250ms-1s'
              WHEN latency_ms < 3000 THEN '1s-3s'
              WHEN latency_ms < 10000 THEN '3s-10s'
              ELSE '10s+'
            END AS label,
            CASE
              WHEN latency_ms < 250 THEN 0
              WHEN latency_ms < 1000 THEN 250
              WHEN latency_ms < 3000 THEN 1000
              WHEN latency_ms < 10000 THEN 3000
              ELSE 10000
            END AS min_ms,
            CASE
              WHEN latency_ms < 250 THEN 249
              WHEN latency_ms < 1000 THEN 999
              WHEN latency_ms < 3000 THEN 2999
              WHEN latency_ms < 10000 THEN 9999
              ELSE NULL
            END AS max_ms,
            COUNT(*) AS request_count,
            SUM(CASE WHEN COALESCE(status_code, 500) >= 400 THEN 1 ELSE 0 END) AS error_count
         FROM request_logs",
    );
    push_request_log_filter_where(&mut query, filters);
    query.push(" GROUP BY sort_order, label, min_ms, max_ms ORDER BY sort_order");
    let rows: Vec<AnalyticsLatencyBucketRow> = query.build_query_as().fetch_all(pool).await?;
    let _ = rows.iter().map(|row| row.sort_order).max();
    Ok(rows
        .into_iter()
        .map(|row| AnalyticsLatencyBucket {
            label: row.label,
            min_ms: row.min_ms,
            max_ms: row.max_ms,
            request_count: row.request_count,
            error_count: row.error_count,
        })
        .collect())
}
