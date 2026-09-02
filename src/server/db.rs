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
