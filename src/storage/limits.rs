use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::auth;

use super::{
    api_keys::{list_api_keys, list_api_keys_for_user},
    db::{now_string, with_immediate_transaction},
    users::list_users,
};

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct LimitPolicy {
    pub scope: String,
    pub subject_id: String,
    pub request_quota: Option<i64>,
    pub request_quota_mode: String,
    pub request_window_seconds: i64,
    pub token_quota: Option<i64>,
    pub token_quota_mode: String,
    pub token_window_seconds: i64,
    pub rate_limit_requests: Option<i64>,
    pub rate_limit_mode: String,
    pub rate_limit_window_seconds: i64,
    pub concurrency_limit: Option<i64>,
    pub concurrency_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LimitPolicyPatch {
    pub request_quota: LimitPatchValue,
    pub request_window_seconds: Option<i64>,
    pub token_quota: LimitPatchValue,
    pub token_window_seconds: Option<i64>,
    pub rate_limit_requests: LimitPatchValue,
    pub rate_limit_window_seconds: Option<i64>,
    pub concurrency_limit: LimitPatchValue,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LimitPatchValue {
    #[default]
    Missing,
    Inherit,
    Clear,
    Set(i64),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitBucketState {
    pub limit: Option<i64>,
    pub used: i64,
    pub remaining: Option<i64>,
    pub window_seconds: Option<i64>,
    pub reset_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConcurrencyState {
    pub limit: Option<i64>,
    pub in_flight: i64,
    pub remaining: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitSubjectState {
    pub scope: String,
    pub subject_id: String,
    pub policy: LimitPolicy,
    pub effective_policy: LimitPolicy,
    pub request_quota: LimitBucketState,
    pub token_budget: LimitBucketState,
    pub rate_limit: LimitBucketState,
    pub concurrency: ConcurrencyState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserLimitState {
    pub user: LimitSubjectState,
    pub current_key: Option<LimitSubjectState>,
    pub api_keys: Vec<LimitSubjectState>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdminLimitState {
    pub system: LimitPolicy,
    pub users: Vec<LimitSubjectState>,
    pub api_keys: Vec<LimitSubjectState>,
}

#[derive(Clone, Debug)]
pub struct LimitAdmission {
    pub usage_event_id: String,
    pub inflight_request_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LimitRejection {
    pub code: &'static str,
    pub message: String,
    pub scope: String,
    pub subject_id: String,
    pub limit_name: &'static str,
    pub limit: i64,
    pub used: i64,
    pub reset_at: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LimitAdmissionError {
    #[error("limit rejected")]
    Rejected(LimitRejection),
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

pub async fn get_limit_policy(
    pool: &SqlitePool,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<Option<LimitPolicy>> {
    sqlx::query_as(
        "SELECT scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
                token_quota, token_quota_mode, token_window_seconds,
                rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
                concurrency_limit, concurrency_mode, created_at, updated_at
         FROM limit_policies
         WHERE scope = ? AND subject_id = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_limit_policy(
    pool: &SqlitePool,
    scope: &str,
    subject_id: &str,
    patch: &LimitPolicyPatch,
) -> sqlx::Result<LimitPolicy> {
    let mut conn = pool.acquire().await?;
    upsert_limit_policy_conn(&mut conn, scope, subject_id, patch).await
}

pub async fn upsert_limit_policy_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
    patch: &LimitPolicyPatch,
) -> sqlx::Result<LimitPolicy> {
    let existing = get_limit_policy_conn(conn, scope, subject_id).await?;
    let base = existing.unwrap_or_else(|| default_policy(scope, subject_id));
    let now = now_string();
    let request_quota = apply_nullable_limit_patch(
        &patch.request_quota,
        base.request_quota,
        &base.request_quota_mode,
        scope,
    );
    let token_quota = apply_nullable_limit_patch(
        &patch.token_quota,
        base.token_quota,
        &base.token_quota_mode,
        scope,
    );
    let rate_limit = apply_nullable_limit_patch(
        &patch.rate_limit_requests,
        base.rate_limit_requests,
        &base.rate_limit_mode,
        scope,
    );
    let concurrency = apply_nullable_limit_patch(
        &patch.concurrency_limit,
        base.concurrency_limit,
        &base.concurrency_mode,
        scope,
    );
    let policy = LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: request_quota.0,
        request_quota_mode: request_quota.1,
        request_window_seconds: patch
            .request_window_seconds
            .unwrap_or(base.request_window_seconds),
        token_quota: token_quota.0,
        token_quota_mode: token_quota.1,
        token_window_seconds: patch
            .token_window_seconds
            .unwrap_or(base.token_window_seconds),
        rate_limit_requests: rate_limit.0,
        rate_limit_mode: rate_limit.1,
        rate_limit_window_seconds: patch
            .rate_limit_window_seconds
            .unwrap_or(base.rate_limit_window_seconds),
        concurrency_limit: concurrency.0,
        concurrency_mode: concurrency.1,
        created_at: base.created_at,
        updated_at: now.clone(),
    };
    sqlx::query(
        "INSERT INTO limit_policies
         (scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
          token_quota, token_quota_mode, token_window_seconds,
          rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
          concurrency_limit, concurrency_mode, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(scope, subject_id) DO UPDATE SET
            request_quota = excluded.request_quota,
            request_quota_mode = excluded.request_quota_mode,
            request_window_seconds = excluded.request_window_seconds,
            token_quota = excluded.token_quota,
            token_quota_mode = excluded.token_quota_mode,
            token_window_seconds = excluded.token_window_seconds,
            rate_limit_requests = excluded.rate_limit_requests,
            rate_limit_mode = excluded.rate_limit_mode,
            rate_limit_window_seconds = excluded.rate_limit_window_seconds,
            concurrency_limit = excluded.concurrency_limit,
            concurrency_mode = excluded.concurrency_mode,
            updated_at = excluded.updated_at",
    )
    .bind(&policy.scope)
    .bind(&policy.subject_id)
    .bind(policy.request_quota)
    .bind(&policy.request_quota_mode)
    .bind(policy.request_window_seconds)
    .bind(policy.token_quota)
    .bind(&policy.token_quota_mode)
    .bind(policy.token_window_seconds)
    .bind(policy.rate_limit_requests)
    .bind(&policy.rate_limit_mode)
    .bind(policy.rate_limit_window_seconds)
    .bind(policy.concurrency_limit)
    .bind(&policy.concurrency_mode)
    .bind(&policy.created_at)
    .bind(&policy.updated_at)
    .execute(&mut *conn)
    .await?;
    get_limit_policy_conn(conn, scope, subject_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn user_limit_state(
    pool: &SqlitePool,
    user_id: &str,
    current_api_key_id: Option<&str>,
) -> sqlx::Result<UserLimitState> {
    let system = system_limit_policy(pool).await?;
    let user_stored_policy = get_limit_policy(pool, "user", user_id).await?;
    let user_policy = merge_policy(&system, user_stored_policy.as_ref(), "user", user_id);
    let user_display_policy = display_policy(user_stored_policy, &user_policy, "user", user_id);
    let user = limit_subject_state(
        pool,
        user_id,
        "user",
        user_id,
        user_display_policy,
        user_policy,
    )
    .await?;
    let keys = list_api_keys_for_user(pool, user_id).await?;
    let mut api_keys = Vec::with_capacity(keys.len());
    let mut current_key = None;
    for key in keys {
        let state = api_key_limit_state(pool, &system, &key.id).await?;
        if current_api_key_id == Some(key.id.as_str()) {
            current_key = Some(state.clone());
        }
        api_keys.push(state);
    }
    Ok(UserLimitState {
        user,
        current_key,
        api_keys,
    })
}

pub async fn admin_limit_state(pool: &SqlitePool) -> sqlx::Result<AdminLimitState> {
    let system = system_limit_policy(pool).await?;
    let users = list_users(pool).await?;
    let keys = list_api_keys(pool).await?;
    let mut user_states = Vec::with_capacity(users.len());
    for user in users {
        let stored_policy = get_limit_policy(pool, "user", &user.id).await?;
        let policy = merge_policy(&system, stored_policy.as_ref(), "user", &user.id);
        let display = display_policy(stored_policy, &policy, "user", &user.id);
        user_states
            .push(limit_subject_state(pool, &user.id, "user", &user.id, display, policy).await?);
    }
    let mut key_states = Vec::with_capacity(keys.len());
    for key in keys {
        key_states.push(api_key_limit_state(pool, &system, &key.id).await?);
    }
    Ok(AdminLimitState {
        system,
        users: user_states,
        api_keys: key_states,
    })
}

pub async fn admit_limited_request(
    pool: &SqlitePool,
    user_id: &str,
    api_key_id: &str,
) -> Result<LimitAdmission, LimitAdmissionError> {
    let user_id = user_id.to_string();
    let api_key_id = api_key_id.to_string();
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move { admit_limited_request_in_tx(conn, &user_id, &api_key_id).await })
    })
    .await
}

pub(super) async fn admit_limited_request_in_tx(
    conn: &mut sqlx::SqliteConnection,
    user_id: &str,
    api_key_id: &str,
) -> Result<LimitAdmission, LimitAdmissionError> {
    let now = Utc::now();
    let now_text = timestamp(now);
    sqlx::query("DELETE FROM limit_inflight_requests WHERE expires_at <= ?")
        .bind(&now_text)
        .execute(&mut *conn)
        .await?;

    let system = system_limit_policy_conn(conn).await?;
    let user_policy = effective_subject_policy_conn(conn, &system, "user", user_id).await?;
    let key_policy = effective_subject_policy_conn(conn, &system, "api_key", api_key_id).await?;
    let scopes = vec![
        EnforcedLimitScope {
            scope: "user",
            subject_id: user_id,
            policy: user_policy,
        },
        EnforcedLimitScope {
            scope: "api_key",
            subject_id: api_key_id,
            policy: key_policy,
        },
    ];

    for scope in &scopes {
        if let Some(rejection) = limit_rejection_for_scope(conn, scope, now).await? {
            return Err(LimitAdmissionError::Rejected(rejection));
        }
    }

    let usage_event_id = auth::new_id();
    sqlx::query(
        "INSERT INTO limit_usage_events
         (id, user_id, api_key_id, request_count, total_tokens, created_at)
         VALUES (?, ?, ?, 1, 0, ?)",
    )
    .bind(&usage_event_id)
    .bind(user_id)
    .bind(api_key_id)
    .bind(&now_text)
    .execute(&mut *conn)
    .await?;

    let inflight_request_id = auth::new_id();
    let expires_at = timestamp(now + Duration::hours(6));
    sqlx::query(
        "INSERT INTO limit_inflight_requests (id, user_id, api_key_id, started_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&inflight_request_id)
    .bind(user_id)
    .bind(api_key_id)
    .bind(&now_text)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;

    for scope in &scopes {
        let window_started_at = rate_window_start(now, scope.policy.rate_limit_window_seconds);
        sqlx::query(
            "INSERT INTO limit_rate_counters
             (scope, subject_id, window_started_at, request_count, updated_at)
             VALUES (?, ?, ?, 1, ?)
             ON CONFLICT(scope, subject_id, window_started_at) DO UPDATE SET
                request_count = request_count + 1,
                updated_at = excluded.updated_at",
        )
        .bind(scope.scope)
        .bind(scope.subject_id)
        .bind(window_started_at)
        .bind(&now_text)
        .execute(&mut *conn)
        .await?;
    }

    Ok(LimitAdmission {
        usage_event_id,
        inflight_request_id,
    })
}

pub async fn finalize_limit_admission(
    pool: &SqlitePool,
    admission: &LimitAdmission,
    total_tokens: i64,
) -> sqlx::Result<()> {
    let usage_event_id = admission.usage_event_id.clone();
    let inflight_request_id = admission.inflight_request_id.clone();
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move {
            let now = now_string();
            sqlx::query(
                "UPDATE limit_usage_events
                 SET total_tokens = ?, finalized_at = ?
                 WHERE id = ?",
            )
            .bind(total_tokens.max(0))
            .bind(&now)
            .bind(&usage_event_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query("DELETE FROM limit_inflight_requests WHERE id = ?")
                .bind(&inflight_request_id)
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
    .await
}

async fn api_key_limit_state(
    pool: &SqlitePool,
    system: &LimitPolicy,
    api_key_id: &str,
) -> sqlx::Result<LimitSubjectState> {
    let stored_policy = get_limit_policy(pool, "api_key", api_key_id).await?;
    let policy = merge_policy(system, stored_policy.as_ref(), "api_key", api_key_id);
    let display = display_policy(stored_policy, &policy, "api_key", api_key_id);
    limit_subject_state(pool, api_key_id, "api_key", api_key_id, display, policy).await
}

async fn system_limit_policy(pool: &SqlitePool) -> sqlx::Result<LimitPolicy> {
    if let Some(policy) = get_limit_policy(pool, "system", "").await? {
        return Ok(policy);
    }
    upsert_limit_policy(pool, "system", "", &LimitPolicyPatch::default()).await
}

async fn system_limit_policy_conn(conn: &mut sqlx::SqliteConnection) -> sqlx::Result<LimitPolicy> {
    get_limit_policy_conn(conn, "system", "")
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

async fn effective_subject_policy_conn(
    conn: &mut sqlx::SqliteConnection,
    system: &LimitPolicy,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<LimitPolicy> {
    let override_policy = get_limit_policy_conn(conn, scope, subject_id).await?;
    Ok(merge_policy(
        system,
        override_policy.as_ref(),
        scope,
        subject_id,
    ))
}

async fn get_limit_policy_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<Option<LimitPolicy>> {
    sqlx::query_as(
        "SELECT scope, subject_id, request_quota, request_quota_mode, request_window_seconds,
                token_quota, token_quota_mode, token_window_seconds,
                rate_limit_requests, rate_limit_mode, rate_limit_window_seconds,
                concurrency_limit, concurrency_mode, created_at, updated_at
         FROM limit_policies
         WHERE scope = ? AND subject_id = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .fetch_optional(&mut *conn)
    .await
}

fn merge_policy(
    system: &LimitPolicy,
    override_policy: Option<&LimitPolicy>,
    scope: &str,
    subject_id: &str,
) -> LimitPolicy {
    let Some(override_policy) = override_policy else {
        let mut policy = system.clone();
        policy.scope = scope.to_string();
        policy.subject_id = subject_id.to_string();
        return policy;
    };
    let request_quota = resolve_nullable_limit(
        system.request_quota,
        &system.request_quota_mode,
        override_policy.request_quota,
        &override_policy.request_quota_mode,
    );
    let token_quota = resolve_nullable_limit(
        system.token_quota,
        &system.token_quota_mode,
        override_policy.token_quota,
        &override_policy.token_quota_mode,
    );
    let rate_limit = resolve_nullable_limit(
        system.rate_limit_requests,
        &system.rate_limit_mode,
        override_policy.rate_limit_requests,
        &override_policy.rate_limit_mode,
    );
    let concurrency = resolve_nullable_limit(
        system.concurrency_limit,
        &system.concurrency_mode,
        override_policy.concurrency_limit,
        &override_policy.concurrency_mode,
    );
    LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: request_quota.0,
        request_quota_mode: request_quota.1,
        request_window_seconds: inherited_window_seconds(
            system.request_window_seconds,
            override_policy.request_window_seconds,
            &override_policy.request_quota_mode,
        ),
        token_quota: token_quota.0,
        token_quota_mode: token_quota.1,
        token_window_seconds: inherited_window_seconds(
            system.token_window_seconds,
            override_policy.token_window_seconds,
            &override_policy.token_quota_mode,
        ),
        rate_limit_requests: rate_limit.0,
        rate_limit_mode: rate_limit.1,
        rate_limit_window_seconds: inherited_window_seconds(
            system.rate_limit_window_seconds,
            override_policy.rate_limit_window_seconds,
            &override_policy.rate_limit_mode,
        ),
        concurrency_limit: concurrency.0,
        concurrency_mode: concurrency.1,
        created_at: override_policy.created_at.clone(),
        updated_at: override_policy.updated_at.clone(),
    }
}

fn display_policy(
    stored_policy: Option<LimitPolicy>,
    effective_policy: &LimitPolicy,
    scope: &str,
    subject_id: &str,
) -> LimitPolicy {
    let mut policy = stored_policy.unwrap_or_else(|| default_policy(scope, subject_id));
    policy.request_window_seconds = inherited_window_seconds(
        effective_policy.request_window_seconds,
        policy.request_window_seconds,
        &policy.request_quota_mode,
    );
    policy.token_window_seconds = inherited_window_seconds(
        effective_policy.token_window_seconds,
        policy.token_window_seconds,
        &policy.token_quota_mode,
    );
    policy.rate_limit_window_seconds = inherited_window_seconds(
        effective_policy.rate_limit_window_seconds,
        policy.rate_limit_window_seconds,
        &policy.rate_limit_mode,
    );
    policy
}

fn inherited_window_seconds(system_window: i64, override_window: i64, mode: &str) -> i64 {
    if mode == "inherit" {
        system_window
    } else {
        override_window
    }
}

fn apply_nullable_limit_patch(
    patch: &LimitPatchValue,
    current_value: Option<i64>,
    current_mode: &str,
    scope: &str,
) -> (Option<i64>, String) {
    match patch {
        LimitPatchValue::Missing => (current_value, current_mode.to_string()),
        LimitPatchValue::Inherit => (None, "inherit".to_string()),
        LimitPatchValue::Clear => (None, "unlimited".to_string()),
        LimitPatchValue::Set(value) => (Some(*value), "limited".to_string()),
    }
    .normalize_system_mode(scope)
}

fn resolve_nullable_limit(
    system_value: Option<i64>,
    system_mode: &str,
    override_value: Option<i64>,
    override_mode: &str,
) -> (Option<i64>, String) {
    match override_mode {
        "inherit" => match system_mode {
            "limited" => (system_value, "limited".to_string()),
            _ => (None, "unlimited".to_string()),
        },
        "limited" => (override_value, "limited".to_string()),
        "unlimited" => (None, "unlimited".to_string()),
        _ => (override_value.or(system_value), "limited".to_string()),
    }
}

trait LimitModeNormalize {
    fn normalize_system_mode(self, scope: &str) -> Self;
}

impl LimitModeNormalize for (Option<i64>, String) {
    fn normalize_system_mode(self, scope: &str) -> Self {
        if scope != "system" || self.1 != "inherit" {
            return self;
        }
        if self.0.is_some() {
            (self.0, "limited".to_string())
        } else {
            (None, "unlimited".to_string())
        }
    }
}

struct EnforcedLimitScope<'a> {
    scope: &'static str,
    subject_id: &'a str,
    policy: LimitPolicy,
}

async fn limit_rejection_for_scope(
    conn: &mut sqlx::SqliteConnection,
    scope: &EnforcedLimitScope<'_>,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<LimitRejection>> {
    if let Some(limit) = scope.policy.request_quota {
        let used = usage_count_conn(
            conn,
            scope.scope,
            scope.subject_id,
            "request_count",
            now,
            scope.policy.request_window_seconds,
        )
        .await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "quota_exceeded",
                message: "request quota exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "request_quota",
                limit,
                used,
                reset_at: Some(timestamp(
                    now + Duration::seconds(scope.policy.request_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.token_quota {
        let used = usage_count_conn(
            conn,
            scope.scope,
            scope.subject_id,
            "total_tokens",
            now,
            scope.policy.token_window_seconds,
        )
        .await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "quota_exceeded",
                message: "token budget exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "token_budget",
                limit,
                used,
                reset_at: Some(timestamp(
                    now + Duration::seconds(scope.policy.token_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.rate_limit_requests {
        let window_started_at = rate_window_start(now, scope.policy.rate_limit_window_seconds);
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(request_count, 0)
             FROM limit_rate_counters
             WHERE scope = ? AND subject_id = ? AND window_started_at = ?",
        )
        .bind(scope.scope)
        .bind(scope.subject_id)
        .bind(&window_started_at)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or_default();
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "rate_limited",
                message: "rate limit exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "rate_limit",
                limit,
                used,
                reset_at: Some(timestamp(
                    parse_timestamp(&window_started_at)
                        + Duration::seconds(scope.policy.rate_limit_window_seconds),
                )),
            }));
        }
    }
    if let Some(limit) = scope.policy.concurrency_limit {
        let used = inflight_count_conn(conn, scope.scope, scope.subject_id).await?;
        if used >= limit {
            return Ok(Some(LimitRejection {
                code: "concurrency_limited",
                message: "concurrent request limit exceeded".to_string(),
                scope: scope.scope.to_string(),
                subject_id: scope.subject_id.to_string(),
                limit_name: "concurrency",
                limit,
                used,
                reset_at: None,
            }));
        }
    }
    Ok(None)
}

async fn limit_subject_state(
    pool: &SqlitePool,
    owner_user_id: &str,
    scope: &str,
    subject_id: &str,
    display_policy: LimitPolicy,
    policy: LimitPolicy,
) -> sqlx::Result<LimitSubjectState> {
    let now = Utc::now();
    let request_used = usage_count(
        pool,
        scope,
        owner_user_id,
        subject_id,
        "request_count",
        now,
        policy.request_window_seconds,
    )
    .await?;
    let token_used = usage_count(
        pool,
        scope,
        owner_user_id,
        subject_id,
        "total_tokens",
        now,
        policy.token_window_seconds,
    )
    .await?;
    let rate_window_started_at = rate_window_start(now, policy.rate_limit_window_seconds);
    let rate_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(request_count, 0)
         FROM limit_rate_counters
         WHERE scope = ? AND subject_id = ? AND window_started_at = ?",
    )
    .bind(scope)
    .bind(subject_id)
    .bind(&rate_window_started_at)
    .fetch_optional(pool)
    .await?
    .unwrap_or_default();
    let in_flight = inflight_count(pool, scope, subject_id).await?;

    Ok(LimitSubjectState {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        effective_policy: policy.clone(),
        request_quota: bucket_state(
            policy.request_quota,
            request_used,
            Some(policy.request_window_seconds),
            Some(timestamp(
                now + Duration::seconds(policy.request_window_seconds),
            )),
        ),
        token_budget: bucket_state(
            policy.token_quota,
            token_used,
            Some(policy.token_window_seconds),
            Some(timestamp(
                now + Duration::seconds(policy.token_window_seconds),
            )),
        ),
        rate_limit: bucket_state(
            policy.rate_limit_requests,
            rate_used,
            Some(policy.rate_limit_window_seconds),
            Some(timestamp(
                parse_timestamp(&rate_window_started_at)
                    + Duration::seconds(policy.rate_limit_window_seconds),
            )),
        ),
        concurrency: ConcurrencyState {
            limit: policy.concurrency_limit,
            in_flight,
            remaining: remaining(policy.concurrency_limit, in_flight),
        },
        policy: display_policy,
    })
}

async fn usage_count(
    pool: &SqlitePool,
    scope: &str,
    owner_user_id: &str,
    subject_id: &str,
    column: &str,
    now: DateTime<Utc>,
    window_seconds: i64,
) -> sqlx::Result<i64> {
    let cutoff = timestamp(now - Duration::seconds(window_seconds.max(1)));
    let sql = match (scope, column) {
        ("user", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("user", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("api_key", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        ("api_key", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        _ => return Ok(0),
    };
    let id = if scope == "user" {
        owner_user_id
    } else {
        subject_id
    };
    sqlx::query_scalar(sql)
        .bind(id)
        .bind(cutoff)
        .fetch_one(pool)
        .await
}

async fn usage_count_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
    column: &str,
    now: DateTime<Utc>,
    window_seconds: i64,
) -> sqlx::Result<i64> {
    let cutoff = timestamp(now - Duration::seconds(window_seconds.max(1)));
    let sql = match (scope, column) {
        ("user", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("user", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE user_id = ? AND created_at >= ?"
        }
        ("api_key", "request_count") => {
            "SELECT COALESCE(SUM(request_count), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        ("api_key", "total_tokens") => {
            "SELECT COALESCE(SUM(total_tokens), 0) FROM limit_usage_events WHERE api_key_id = ? AND created_at >= ?"
        }
        _ => return Ok(0),
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .bind(cutoff)
        .fetch_one(&mut *conn)
        .await
}

async fn inflight_count(pool: &SqlitePool, scope: &str, subject_id: &str) -> sqlx::Result<i64> {
    let sql = if scope == "api_key" {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE api_key_id = ?"
    } else {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE user_id = ?"
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .fetch_one(pool)
        .await
}

async fn inflight_count_conn(
    conn: &mut sqlx::SqliteConnection,
    scope: &str,
    subject_id: &str,
) -> sqlx::Result<i64> {
    let sql = if scope == "api_key" {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE api_key_id = ?"
    } else {
        "SELECT COUNT(*) FROM limit_inflight_requests WHERE user_id = ?"
    };
    sqlx::query_scalar(sql)
        .bind(subject_id)
        .fetch_one(&mut *conn)
        .await
}

fn bucket_state(
    limit: Option<i64>,
    used: i64,
    window_seconds: Option<i64>,
    reset_at: Option<String>,
) -> LimitBucketState {
    LimitBucketState {
        limit,
        used,
        remaining: remaining(limit, used),
        window_seconds,
        reset_at,
    }
}

fn remaining(limit: Option<i64>, used: i64) -> Option<i64> {
    limit.map(|limit| (limit - used).max(0))
}

fn default_policy(scope: &str, subject_id: &str) -> LimitPolicy {
    let now = now_string();
    let nullable_mode = if scope == "system" {
        "unlimited"
    } else {
        "inherit"
    };
    LimitPolicy {
        scope: scope.to_string(),
        subject_id: subject_id.to_string(),
        request_quota: None,
        request_quota_mode: nullable_mode.to_string(),
        request_window_seconds: 86_400,
        token_quota: None,
        token_quota_mode: nullable_mode.to_string(),
        token_window_seconds: 86_400,
        rate_limit_requests: None,
        rate_limit_mode: nullable_mode.to_string(),
        rate_limit_window_seconds: 60,
        concurrency_limit: None,
        concurrency_mode: nullable_mode.to_string(),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn rate_window_start(now: DateTime<Utc>, window_seconds: i64) -> String {
    let window_seconds = window_seconds.max(1);
    let timestamp = now.timestamp();
    let start = timestamp - timestamp.rem_euclid(window_seconds);
    DateTime::<Utc>::from_timestamp(start, 0)
        .unwrap_or(now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
