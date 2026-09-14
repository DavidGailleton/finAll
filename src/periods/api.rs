//! The server function the browser calls to resolve a period preset.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::periods::types::{PeriodPreset, PeriodRangeDto};

/// Resolve `preset` against the server's current date.
///
/// Deliberately explicit and on-demand (called only when the visitor picks a
/// preset), never read during initial/shared rendering — see
/// `src/CLAUDE.md`'s rule against nondeterministic shared rendering. The
/// server's clock is used rather than the browser's so every visitor's
/// "today" agrees with the account the report itself is computed against.
#[server]
pub async fn resolve_period(preset: PeriodPreset) -> Result<PeriodRangeDto, ServerFnError> {
    use sqlx::types::chrono::Utc;

    use crate::server::auth::extract;
    use crate::server::periods::{self, PeriodError};

    let pool = expect_context::<sqlx::PgPool>();

    extract::current_user(&pool)
        .await?
        .ok_or(PeriodError::Unauthorized)?;

    let today = Utc::now().date_naive();
    let (from, to) = periods::resolve(preset, today);

    Ok(PeriodRangeDto {
        from: from.format("%Y-%m-%d").to_string(),
        to: to.format("%Y-%m-%d").to_string(),
    })
}
