use std::{path::Path, str::FromStr};

use anyhow::Context;
use chrono::Utc;
use futures_util::future::BoxFuture;
use sqlx::{
    Connection, QueryBuilder, Sqlite, SqlitePool,
    pool::PoolConnection,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<SqlitePool> {
    create_sqlite_parent(database_url)?;
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("parsing database URL {database_url}"))?
        .create_if_missing(true)
        .foreign_keys(true);
    let max_connections = if database_url.contains(":memory:") {
        1
    } else {
        5
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .context("connecting SQLite database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running database migrations")?;
    clear_stale_limit_inflight(&pool).await?;
    Ok(pool)
}

fn create_sqlite_parent(database_url: &str) -> anyhow::Result<()> {
    let Some(path) = database_url.strip_prefix("sqlite://") else {
        return Ok(());
    };
    if path == ":memory:" || path.starts_with("file:") {
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating SQLite database directory {}", parent.display()))?;
    }
    Ok(())
}

struct ImmediateTransactionConnection {
    connection: PoolConnection<Sqlite>,
    reusable: bool,
}

impl ImmediateTransactionConnection {
    fn new(connection: PoolConnection<Sqlite>) -> Self {
        Self {
            connection,
            reusable: false,
        }
    }

    fn connection(&mut self) -> &mut sqlx::SqliteConnection {
        &mut self.connection
    }

    fn mark_reusable(&mut self) {
        self.reusable = true;
    }
}

impl Drop for ImmediateTransactionConnection {
    fn drop(&mut self) {
        if !self.reusable {
            self.connection.close_on_drop();
        }
    }
}

pub(super) async fn with_immediate_transaction<T, E, F>(
    pool: &SqlitePool,
    operation: F,
) -> Result<T, E>
where
    E: From<sqlx::Error>,
    F: for<'connection> FnOnce(
        &'connection mut sqlx::SqliteConnection,
    ) -> BoxFuture<'connection, Result<T, E>>,
{
    let mut connection =
        ImmediateTransactionConnection::new(pool.acquire().await.map_err(E::from)?);
    let mut tx = connection
        .connection()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(E::from)?;
    let result = match operation(&mut tx).await {
        Ok(result) => result,
        Err(error) => {
            return match tx.rollback().await {
                Ok(()) => {
                    connection.mark_reusable();
                    Err(error)
                }
                Err(rollback_error) => Err(E::from(rollback_error)),
            };
        }
    };
    match tx.commit().await {
        Ok(()) => {
            connection.mark_reusable();
            Ok(result)
        }
        Err(error) => Err(E::from(error)),
    }
}

pub(super) fn push_where(query: &mut QueryBuilder<'_, Sqlite>, has_where: &mut bool) {
    if *has_where {
        query.push(" AND ");
    } else {
        query.push(" WHERE ");
        *has_where = true;
    }
}

async fn clear_stale_limit_inflight(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM limit_inflight_requests")
        .execute(pool)
        .await?;
    Ok(())
}
pub fn now_string() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

pub(crate) fn sqlite_bool(value: i64) -> anyhow::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => anyhow::bail!("invalid SQLite boolean"),
    }
}
