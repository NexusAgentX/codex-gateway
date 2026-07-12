use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::{auth, config::Config};

use super::{api_keys::create_or_replace_named_key, db::now_string};

pub async fn seed_bootstrap_admin(pool: &SqlitePool, config: &Config) -> anyhow::Result<()> {
    let Some(email) = &config.admin_email else {
        return Ok(());
    };
    let user_id = ensure_bootstrap_admin(
        pool,
        email,
        config.admin_password.as_deref(),
        Some("Bootstrap Admin"),
    )
    .await?;

    if let Some(key) = &config.bootstrap_admin_key {
        let prepared = auth::prepare_existing_api_key(&config.app_secret, key)?;
        create_or_replace_named_key(
            pool,
            &user_id,
            "bootstrap-admin",
            &prepared.prefix,
            &prepared.hash,
        )
        .await?;
    }

    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub display_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Clone, FromRow)]
pub struct UserCredentialsRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub status: String,
}

pub type UserCredentials = UserCredentialsRecord;

#[derive(Clone, Debug)]
pub struct CreateUser {
    pub email: String,
    pub password: String,
    pub role: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UpdateUser {
    pub role: Option<String>,
    pub status: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResetPassword {
    pub password: String,
}

pub async fn ensure_bootstrap_admin(
    pool: &SqlitePool,
    email: &str,
    password: Option<&str>,
    display_name: Option<&str>,
) -> anyhow::Result<String> {
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    let now = now_string();
    if let Some((id,)) = existing {
        if let Some(password) = password {
            let password_hash = auth::hash_password(password)?;
            sqlx::query(
                "UPDATE users
                 SET password_hash = ?, role = 'admin', status = 'active',
                     display_name = COALESCE(?, display_name), updated_at = ?
                 WHERE id = ?",
            )
            .bind(password_hash)
            .bind(display_name)
            .bind(&now)
            .bind(&id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE users
                 SET role = 'admin', status = 'active',
                     display_name = COALESCE(?, display_name), updated_at = ?
                 WHERE id = ?",
            )
            .bind(display_name)
            .bind(&now)
            .bind(&id)
            .execute(pool)
            .await?;
        }
        return Ok(id);
    }

    let password_hash = auth::hash_password(password.unwrap_or("change-me-on-first-login"))?;
    let id = auth::new_id();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, status, display_name, created_at, updated_at)
         VALUES (?, ?, ?, 'admin', 'active', ?, ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn ensure_user(pool: &SqlitePool, input: &CreateUser) -> anyhow::Result<String> {
    let mut conn = pool.acquire().await?;
    ensure_user_conn(&mut conn, input).await
}

pub async fn ensure_user_conn(
    conn: &mut sqlx::SqliteConnection,
    input: &CreateUser,
) -> anyhow::Result<String> {
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(&input.email)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some((id,)) = existing {
        return Ok(id);
    }

    let id = auth::new_id();
    let password_hash = auth::hash_password(&input.password)?;
    let now = now_string();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, status, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'active', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.email)
    .bind(password_hash)
    .bind(&input.role)
    .bind(&input.display_name)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

pub async fn get_user(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as("SELECT id, email, role, status, display_name, created_at, updated_at, last_login_at FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_user(
    pool: &SqlitePool,
    id: &str,
    input: &UpdateUser,
) -> sqlx::Result<Option<User>> {
    let mut conn = pool.acquire().await?;
    update_user_conn(&mut conn, id, input).await
}

pub async fn update_user_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    input: &UpdateUser,
) -> sqlx::Result<Option<User>> {
    let Some(existing) = get_user_conn(conn, id).await? else {
        return Ok(None);
    };
    let role = input.role.as_deref().unwrap_or(&existing.role);
    let status = input.status.as_deref().unwrap_or(&existing.status);
    let display_name = input
        .display_name
        .as_ref()
        .or(existing.display_name.as_ref());
    let now = now_string();
    sqlx::query(
        "UPDATE users
         SET role = ?, status = ?, display_name = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(role)
    .bind(status)
    .bind(display_name)
    .bind(&now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    get_user_conn(conn, id).await
}

async fn get_user_conn(conn: &mut sqlx::SqliteConnection, id: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as("SELECT id, email, role, status, display_name, created_at, updated_at, last_login_at FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

pub async fn reset_user_password(
    pool: &SqlitePool,
    id: &str,
    password: &str,
) -> anyhow::Result<bool> {
    let mut conn = pool.acquire().await?;
    reset_user_password_conn(&mut conn, id, password).await
}

pub async fn reset_user_password_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    password: &str,
) -> anyhow::Result<bool> {
    let command = ResetPassword {
        password: password.to_string(),
    };
    reset_user_password_command_conn(conn, id, &command).await
}

pub(crate) async fn reset_user_password_command_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    command: &ResetPassword,
) -> anyhow::Result<bool> {
    let password_hash = auth::hash_password(&command.password)?;
    let result = sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(password_hash)
        .bind(now_string())
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_users(pool: &SqlitePool) -> sqlx::Result<Vec<User>> {
    sqlx::query_as("SELECT id, email, role, status, display_name, created_at, updated_at, last_login_at FROM users ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

pub async fn find_user_credentials_by_email(
    pool: &SqlitePool,
    email: &str,
) -> sqlx::Result<Option<UserCredentials>> {
    sqlx::query_as("SELECT id, email, password_hash, role, status FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn mark_user_login(pool: &SqlitePool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_login_at = ?, updated_at = ? WHERE id = ?")
        .bind(now_string())
        .bind(now_string())
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
