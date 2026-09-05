#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use fin_all::app::*;
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    // Generate the list of routes in your Leptos App
    let routes = generate_route_list(App);

    // Open the database pool once and hand it to server functions via context.
    let pool = fin_all::server::db::create_pool()
        .await
        .expect("could not connect to the database (check DATABASE_URL)");

    // Apply pending migrations before serving only when explicitly opted in with
    // `RUN_MIGRATIONS=1` (or `true`). Off by default; otherwise run
    // `sqlx migrate run` out of band.
    let run_migrations = std::env::var("RUN_MIGRATIONS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false);
    if run_migrations {
        fin_all::server::db::run_pending_migrations(&pool)
            .await
            .expect("failed to apply database migrations");
        log!("database migrations applied");
    }

    // Periodically sweep sessions that expired without a logout ever revoking
    // them (see server::auth::session::revoke).
    tokio::spawn({
        let pool = pool.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                interval.tick().await;
                match fin_all::server::auth::session::prune_expired(&pool).await {
                    Ok(count) if count > 0 => {
                        log!("session cleanup: removed {count} expired session(s)")
                    }
                    Ok(_) => {}
                    Err(_) => log!("session cleanup: failed to prune expired sessions"),
                }
            }
        }
    });

    let app = Router::new()
        .route(
            "/health",
            axum::routing::get({
                let pool = pool.clone();
                move || health(pool.clone())
            }),
        )
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            {
                let pool = pool.clone();
                move || provide_context(pool.clone())
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    log!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

/// Liveness and database-readiness probe backing the `/health` endpoint that the
/// Docker healthchecks poll. Returns `200 OK` when a query against the pool
/// succeeds, `503 Service Unavailable` otherwise. The underlying error is logged
/// server-side and never included in the response.
#[cfg(feature = "ssr")]
async fn health(pool: sqlx::PgPool) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    use leptos::logging::log;

    match fin_all::server::db::check_connection(&pool).await {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            log!("health: database readiness check failed: {err}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
