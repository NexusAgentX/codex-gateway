use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::auth;

use super::db::{bool_to_i64, now_string, with_immediate_transaction};

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct Model {
    pub id: String,
    pub public_name: String,
    pub description: Option<String>,
    pub enabled: i64,
    pub visible_to_users: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct UpsertModel {
    pub public_name: String,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub visible_to_users: Option<bool>,
    pub upstream_mappings: Option<Vec<UpsertModelMapping>>,
}

#[derive(Clone, Debug)]
pub struct UpdateModel {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub visible_to_users: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct UpsertModelMapping {
    pub upstream_id: String,
    pub upstream_model_name: String,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct UpstreamModel {
    pub id: String,
    pub model_id: String,
    pub upstream_id: String,
    pub upstream_model_name: String,
    pub enabled: i64,
    pub priority: i64,
    pub weight: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct UpdateModelMapping {
    pub upstream_id: Option<String>,
    pub upstream_model_name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
}

pub async fn list_models(pool: &SqlitePool) -> sqlx::Result<Vec<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models ORDER BY public_name")
        .fetch_all(pool)
        .await
}

pub async fn list_visible_models(pool: &SqlitePool) -> sqlx::Result<Vec<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models WHERE enabled = 1 AND visible_to_users = 1 ORDER BY public_name")
        .fetch_all(pool)
        .await
}

pub async fn create_model(pool: &SqlitePool, input: &UpsertModel) -> sqlx::Result<Model> {
    let input = input.clone();
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move { create_model_conn(conn, &input).await })
    })
    .await
}

pub async fn create_model_conn(
    conn: &mut sqlx::SqliteConnection,
    input: &UpsertModel,
) -> sqlx::Result<Model> {
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO models (id, public_name, description, enabled, visible_to_users, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.public_name)
    .bind(&input.description)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(bool_to_i64(input.visible_to_users.unwrap_or(true)))
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;

    if let Some(mappings) = &input.upstream_mappings {
        for mapping in mappings {
            create_upstream_model_conn(conn, &id, mapping).await?;
        }
    }

    get_model_conn(conn, &id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_model(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn get_model_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> sqlx::Result<Option<Model>> {
    sqlx::query_as("SELECT id, public_name, description, enabled, visible_to_users, created_at, updated_at FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

pub async fn update_model(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateModel,
) -> sqlx::Result<Option<Model>> {
    let mut conn = pool.acquire().await?;
    update_model_conn(&mut conn, id, input).await
}

pub async fn update_model_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    input: &UpdateModel,
) -> sqlx::Result<Option<Model>> {
    let Some(existing) = get_model_conn(conn, id).await? else {
        return Ok(None);
    };
    let description = input.description.as_ref().or(existing.description.as_ref());
    let enabled = input.enabled.map(bool_to_i64).unwrap_or(existing.enabled);
    let visible_to_users = input
        .visible_to_users
        .map(bool_to_i64)
        .unwrap_or(existing.visible_to_users);
    let now = now_string();
    sqlx::query(
        "UPDATE models
         SET description = ?, enabled = ?, visible_to_users = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(description)
    .bind(enabled)
    .bind(visible_to_users)
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    get_model_conn(conn, id).await
}

pub async fn create_upstream_model(
    pool: &SqlitePool,
    model_id: &str,
    input: &UpsertModelMapping,
) -> sqlx::Result<UpstreamModel> {
    let mut conn = pool.acquire().await?;
    create_upstream_model_conn(&mut conn, model_id, input).await
}

pub async fn create_upstream_model_conn(
    conn: &mut sqlx::SqliteConnection,
    model_id: &str,
    input: &UpsertModelMapping,
) -> sqlx::Result<UpstreamModel> {
    let id = auth::new_id();
    let now = now_string();
    sqlx::query(
        "INSERT INTO upstream_models
         (id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(model_id)
    .bind(&input.upstream_id)
    .bind(&input.upstream_model_name)
    .bind(bool_to_i64(input.enabled.unwrap_or(true)))
    .bind(input.priority.unwrap_or(100))
    .bind(input.weight.unwrap_or(1).max(1))
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE id = ?")
        .bind(id)
        .fetch_one(&mut *conn)
        .await
}

pub async fn get_upstream_model(
    pool: &SqlitePool,
    id: &str,
) -> sqlx::Result<Option<UpstreamModel>> {
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_upstream_models_for_model(
    pool: &SqlitePool,
    model_id: &str,
) -> sqlx::Result<Vec<UpstreamModel>> {
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE model_id = ? ORDER BY priority, id")
        .bind(model_id)
        .fetch_all(pool)
        .await
}

pub async fn update_upstream_model(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateModelMapping,
) -> sqlx::Result<Option<UpstreamModel>> {
    let mut conn = pool.acquire().await?;
    update_upstream_model_conn(&mut conn, id, input).await
}

pub async fn update_upstream_model_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    input: &UpdateModelMapping,
) -> sqlx::Result<Option<UpstreamModel>> {
    let Some(existing) = get_upstream_model_conn(conn, id).await? else {
        return Ok(None);
    };
    let upstream_id = input
        .upstream_id
        .as_deref()
        .unwrap_or(&existing.upstream_id);
    let upstream_model_name = input
        .upstream_model_name
        .as_deref()
        .unwrap_or(&existing.upstream_model_name);
    let enabled = input.enabled.map(bool_to_i64).unwrap_or(existing.enabled);
    let priority = input.priority.unwrap_or(existing.priority);
    let weight = input.weight.unwrap_or(existing.weight).max(1);
    let now = now_string();
    sqlx::query(
        "UPDATE upstream_models
         SET upstream_id = ?, upstream_model_name = ?, enabled = ?,
             priority = ?, weight = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(upstream_id)
    .bind(upstream_model_name)
    .bind(enabled)
    .bind(priority)
    .bind(weight)
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    get_upstream_model_conn(conn, id).await
}

async fn get_upstream_model_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> sqlx::Result<Option<UpstreamModel>> {
    sqlx::query_as("SELECT id, model_id, upstream_id, upstream_model_name, enabled, priority, weight, created_at, updated_at FROM upstream_models WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}
