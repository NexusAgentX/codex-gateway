use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::auth;

use super::db::{now_string, with_immediate_transaction};

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct AdminAuditLog {
    pub id: String,
    pub actor_user_id: String,
    pub actor_email: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub status: String,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AdminAuditInsert {
    pub actor_user_id: String,
    pub actor_email: String,
    pub action: &'static str,
    pub resource_type: &'static str,
    pub resource_id: Option<String>,
    pub status: &'static str,
    pub metadata_json: Option<String>,
}
pub async fn with_admin_audit<T, E, F>(pool: &SqlitePool, operation: F) -> Result<T, E>
where
    T: Send,
    E: From<sqlx::Error> + Send,
    F: for<'connection> FnOnce(
            &'connection mut sqlx::SqliteConnection,
        ) -> BoxFuture<'connection, Result<(T, AdminAuditInsert), E>>
        + Send,
    F: 'static,
{
    with_immediate_transaction(pool, move |conn| {
        Box::pin(async move {
            let (result, audit) = operation(conn).await?;
            insert_admin_audit_log_conn(conn, audit)
                .await
                .map_err(E::from)?;
            Ok(result)
        })
    })
    .await
}

pub async fn insert_admin_audit_log(pool: &SqlitePool, log: AdminAuditInsert) -> sqlx::Result<()> {
    let mut conn = pool.acquire().await?;
    insert_admin_audit_log_conn(&mut conn, log).await
}

async fn insert_admin_audit_log_conn(
    conn: &mut sqlx::SqliteConnection,
    log: AdminAuditInsert,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO admin_audit_logs
         (id, actor_user_id, actor_email, action, resource_type, resource_id, status, metadata_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(auth::new_id())
    .bind(log.actor_user_id)
    .bind(log.actor_email)
    .bind(log.action)
    .bind(log.resource_type)
    .bind(log.resource_id)
    .bind(log.status)
    .bind(log.metadata_json)
    .bind(now_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn list_admin_audit_logs(pool: &SqlitePool) -> sqlx::Result<Vec<AdminAuditLog>> {
    sqlx::query_as(
        "SELECT id, actor_user_id, actor_email, action, resource_type, resource_id, status, metadata_json, created_at
         FROM admin_audit_logs
         ORDER BY created_at DESC
         LIMIT 500",
    )
    .fetch_all(pool)
    .await
}
