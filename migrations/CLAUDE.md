# migrations/ — Database and SQL

Applies to `migrations/*.sql` and any schema/query work. See the root `CLAUDE.md` for
scope/workflow/security rules.

## Database

Schema lives in `migrations/*.sql` (sqlx migration format). Current tables: `users`, `sessions`,
`assets` + `fiat_assets` (a per-class detail table; a "currency" is a `fiat`-class asset),
`asset_rates`, `accounts`, `categories`, `merchants`, `transactions`, `transfers`. Amounts are
`NUMERIC(38, 18)`; all tables use `uuidv7()` PKs and soft-delete via `deleted_at`.

Migrations are applied on startup **only** when `RUN_MIGRATIONS=1` (or `true`) is set —
`main.rs` then calls `server::db::run_pending_migrations`, which runs `sqlx::migrate!().run(&pool)`.
The flag is off by default; the normal path is to apply migrations out of band with
`sqlx migrate run` (`make migrate` in dev). `sqlx::migrate!` embeds the SQL at compile time.

SQL is linted with **sqlfluff** (`.sqlfluff`, postgres dialect): keywords/types UPPER,
functions/identifiers lower, 100-col lines. `DATABASE_URL` is required for `sqlx` compile-time
query checks and for `sqlx-cli`.

## Database and migration rules

PostgreSQL and `sqlx` are the established persistence technologies — do not add another database,
ORM, or query layer without explicit approval.

- Treat SQL parameters as untrusted input; use parameterized queries.
- Preserve `NUMERIC(38, 18)` precision and UUIDv7 identifiers unless a schema change explicitly
  requires otherwise. Respect `deleted_at` soft deletion — never silently hard-delete financial
  records.
- Follow existing naming and SQL formatting conventions (SQLFluff config above). Never run
  `sqlfluff fix` automatically — it can rewrite more than the requested change.
- Do not edit a migration that may already have been applied unless explicitly directed. Prefer a
  new migration for an approved schema change. Do not create or run a migration without approval.
- `RUN_MIGRATIONS` stays opt-in and off by default — do not make it run automatically, and do not
  add further automatic migration execution unless explicitly requested.
- Do not treat compile-time `sqlx` query validation as proof that runtime authorization and
  business validation are correct.
- Database operations affecting multiple financial records should use an explicit transaction
  when atomicity is required. Do not invent transaction boundaries; ask when they are unclear.
