use crate::config::DatabaseSection;
use anyhow::{Context, Result};
use sqlx::{
    Pool, Postgres,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

pub type PgStorePool = Pool<Postgres>;

pub const ISSUE_SCHEMA_VERSION: i32 = 1;
pub const PATCH_SCHEMA_VERSION: i32 = 1;
pub const TASK_SCHEMA_VERSION: i32 = 1;
pub const TASK_STATUS_LOG_SCHEMA_VERSION: i32 = 1;
pub const USER_SCHEMA_VERSION: i32 = 1;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Establish a Postgres connection pool using the provided configuration.
///
/// Returns `Ok(None)` when no database URL is configured, allowing callers to
/// continue using the in-memory store in development environments.
pub async fn init_pool(config: &DatabaseSection) -> Result<Option<PgStorePool>> {
    let Some(database_url) = config.database_url() else {
        return Ok(None);
    };

    let max_connections = config.max_connections.max(1);
    let min_connections = config.min_connections.min(max_connections);

    let mut pool_options = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

    let connect_options = PgConnectOptions::from_str(&database_url)
        .context("failed to parse database URL for Postgres pool")?;

    if let Some(idle_timeout_secs) = config.idle_timeout() {
        pool_options = pool_options.idle_timeout(Duration::from_secs(idle_timeout_secs));
    }

    let pool = pool_options
        .connect_with(connect_options)
        .await
        .context("failed to connect to configured Postgres database")?;

    Ok(Some(pool))
}

/// Run embedded SQLx migrations against the provided pool.
pub async fn run_migrations(pool: &PgStorePool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("failed to apply Postgres migrations")
}
