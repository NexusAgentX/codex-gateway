use anyhow::Context;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::auth;

use super::db::now_string;

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct ApiKeySummary {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_prefix: String,
    pub status: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateApiKey {
    pub name: String,
    pub expires_at: Option<String>,
}

pub async fn create_api_key(
    pool: &SqlitePool,
    app_secret: &str,
    user_id: &str,
    input: &CreateApiKey,
) -> anyhow::Result<(ApiKeySummary, String)> {
    let mut conn = pool.acquire().await?;
    create_api_key_conn(&mut conn, app_secret, user_id, input).await
}

pub async fn create_api_key_conn(
    conn: &mut sqlx::SqliteConnection,
    app_secret: &str,
    user_id: &str,
    input: &CreateApiKey,
) -> anyhow::Result<(ApiKeySummary, String)> {
    let prepared = auth::generate_api_key(app_secret);
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, status, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&input.name)
    .bind(&prepared.prefix)
    .bind(&prepared.hash)
    .bind(&input.expires_at)
    .bind(&now)
    .execute(&mut *conn)
    .await?;

    let summary = get_api_key_conn(conn, &id)
        .await?
        .context("created API key not found")?;
    Ok((summary, prepared.plaintext))
}

pub(super) async fn create_or_replace_named_key(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
    prefix: &str,
    hash: &str,
) -> anyhow::Result<()> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM api_keys WHERE user_id = ? AND name = ? LIMIT 1")
            .bind(user_id)
            .bind(name)
            .fetch_optional(pool)
            .await?;
    let now = now_string();
    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE api_keys
             SET key_prefix = ?, key_hash = ?, status = 'active',
                 expires_at = NULL, revoked_at = NULL
             WHERE id = ?",
        )
        .bind(prefix)
        .bind(hash)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        let id = auth::new_id();
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, status, created_at)
             VALUES (?, ?, ?, ?, ?, 'active', ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(prefix)
        .bind(hash)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn get_api_key(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn get_api_key_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> sqlx::Result<Option<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

pub async fn list_api_keys_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> sqlx::Result<Vec<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys WHERE user_id = ? ORDER BY created_at DESC")
        .bind(user_id)
        .fetch_all(pool)
        .await
}

pub async fn list_api_keys(pool: &SqlitePool) -> sqlx::Result<Vec<ApiKeySummary>> {
    sqlx::query_as("SELECT id, user_id, name, key_prefix, status, last_used_at, expires_at, created_at, revoked_at FROM api_keys ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

pub async fn set_api_key_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
) -> sqlx::Result<Option<ApiKeySummary>> {
    let mut conn = pool.acquire().await?;
    set_api_key_status_conn(&mut conn, id, status).await
}

pub async fn set_api_key_status_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    status: &str,
) -> sqlx::Result<Option<ApiKeySummary>> {
    let revoked_at = if status == "revoked" {
        Some(now_string())
    } else {
        None
    };
    sqlx::query(
        "UPDATE api_keys
         SET status = ?, revoked_at = COALESCE(?, revoked_at)
         WHERE id = ?",
    )
    .bind(status)
    .bind(revoked_at)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    get_api_key_conn(conn, id).await
}
