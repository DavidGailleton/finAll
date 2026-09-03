//! Database connection pool.

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Open a connection pool to Postgres using the `DATABASE_URL` environment
/// variable.
///
/// Called once at startup from `main`. The pool is then handed to server
/// functions through Leptos context.
pub async fn create_pool() -> Result<PgPool, sqlx::Error> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| sqlx::Error::Configuration("DATABASE_URL is not set".into()))?;

    PgPoolOptions::new().connect(&url).await
}

/// Apply any pending migrations from `./migrations`, embedded into the binary at
/// compile time by `sqlx::migrate!`.
///
/// Called from `main` at startup only when opted in via the `RUN_MIGRATIONS`
/// environment variable; otherwise migrations are applied out of band with
/// `sqlx migrate run`. `sqlx` acquires a Postgres advisory lock for the
/// duration, so this is safe to run on every boot.
pub async fn run_pending_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!().run(pool).await
}
