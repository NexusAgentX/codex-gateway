use sqlx::{FromRow, SqlitePool};

#[derive(Clone, FromRow)]
pub(super) struct ApiKeyRecord {
    pub api_key_id: String,
    pub user_id: String,
    pub key_hash: String,
    pub key_status: String,
    pub expires_at: Option<String>,
    pub email: String,
    pub role: String,
    pub user_status: String,
}

pub(super) trait AuthPersistence {
    async fn find_api_key_by_prefix(&self, prefix: &str) -> sqlx::Result<Option<ApiKeyRecord>>;

    async fn mark_api_key_used(&self, api_key_id: &str, used_at: &str) -> sqlx::Result<()>;
}

impl AuthPersistence for SqlitePool {
    async fn find_api_key_by_prefix(&self, prefix: &str) -> sqlx::Result<Option<ApiKeyRecord>> {
        sqlx::query_as(
            "SELECT
                api_keys.id AS api_key_id,
                api_keys.user_id AS user_id,
                api_keys.key_hash AS key_hash,
                api_keys.status AS key_status,
                api_keys.expires_at AS expires_at,
                users.email AS email,
                users.role AS role,
                users.status AS user_status
             FROM api_keys
             JOIN users ON users.id = api_keys.user_id
             WHERE api_keys.key_prefix = ?",
        )
        .bind(prefix)
        .fetch_optional(self)
        .await
    }

    async fn mark_api_key_used(&self, api_key_id: &str, used_at: &str) -> sqlx::Result<()> {
        sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
            .bind(used_at)
            .bind(api_key_id)
            .execute(self)
            .await?;
        Ok(())
    }
}
