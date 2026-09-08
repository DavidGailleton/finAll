# src/ — Architecture, Rust, Leptos

Applies to everything under `src/`. See the root `CLAUDE.md` for scope, workflow, and security
rules; see `src/server/CLAUDE.md` for financial-domain rules once you're inside `src/server/`.

## Architecture

- **Single crate, two compile targets**, controlled by mutually exclusive Cargo features:
  - `ssr` — server binary: pulls in `axum`, `tokio`, `leptos_axum`, `sqlx`. Entry point
    `src/main.rs` (guarded by `#[cfg(feature = "ssr")]`).
  - `hydrate` — browser WASM: entry point `hydrate()` in `src/lib.rs`.
  Server-only code (DB access, secrets, Axum handlers) **must** be behind `#[cfg(feature = "ssr")]`
  or inside a `#[server]` function, or it will break the WASM build.
- **Module pattern**: each domain has a shared top-level module with DTOs and `#[server]` fn
  signatures (compiled for both targets), and a matching `src/server/<domain>/` module holding the
  actual DB/business logic. The whole `src/server/` tree is `ssr`-only via one blanket
  `#[cfg(feature = "ssr")] pub mod server;` in `lib.rs` — no per-file `cfg` needed inside it.
  - `src/assets/` + `src/server/assets/` — currency/financial-asset domain.
  - `src/auth/` + `src/server/auth/` — authentication domain.
- `src/pages/`, `src/components/` — shared UI, routed pages and reusable components; no financial
  or DB logic here.
- `src/app.rs` — shared `shell()` (HTML document) + root `App` component/router; runs on both
  targets.
- `src/main.rs` — `ssr`-only server bootstrap (Axum + `leptos_axum`).
- `src/lib.rs` — target entry point and module wiring; the two `#[cfg(feature = ...)]` gates live
  here.
- Package is `fin-all`; Rust imports it as `fin_all` (e.g. `use fin_all::app::*`). Bundle serves
  at `/pkg/fin-all.{js,wasm,css}`.
- `style/main.scss` compiles via `cargo-leptos` (dart-sass + Lightning CSS). `site/` is generated
  build output — do not edit it.

## Rust implementation rules

Write idiomatic stable Rust compatible with the toolchain and dependencies already configured.

- Prefer straightforward code over clever abstractions; keep each function focused on one
  responsibility.
- Follow existing naming, module, visibility, and error-handling conventions. Keep visibility as
  narrow as possible. Preserve existing signatures unless changing one is part of the request.
- Respect ownership/borrowing instead of cloning unnecessarily; avoid unnecessary allocations.
- Do not introduce traits, generics, macros, or wrapper types without a concrete need.
- Do not suppress warnings without explaining why; no `#[allow(...)]` merely to hide a warning
  caused by new code.
- Do not use `unsafe` unless explicitly requested and justified. Do not use unstable Rust features
  unless the project already requires them.
- Do not expose server-only types through code compiled for hydration.
- **In production paths, never introduce**: `unwrap()`, `expect()`, `panic!()`, silently ignored
  errors, or placeholder error handling. Use the project's existing error strategy; if none exists
  and the work needs one, ask before introducing it.

## Leptos and full-stack boundaries

Code in shared components/pages may execute during SSR and in the browser after hydration.
Therefore:

- Do not access PostgreSQL directly from a component; do not expose `sqlx` types to hydration
  code; do not expose secrets/env vars/server internals to WASM. Keep DB and privileged operations
  behind the `ssr` feature and use Leptos server functions when client components need
  server-side behavior. Treat every server function argument as untrusted input.
- Do not assume browser-only APIs are available during SSR, or server-only APIs during hydration.
  Avoid hydration mismatches from nondeterministic rendering — do not read the current time,
  random values, or environment-dependent values during shared initial rendering unless the
  behavior is deliberately coordinated between server and client.
- Follow the router and signal patterns already used by the project. Do not introduce global
  state or a new state-management approach without approval.
- UI: use semantic HTML, associate labels with form controls, preserve keyboard accessibility,
  use buttons for actions, and don't make non-interactive elements clickable without proper
  semantics. Don't add unrelated loading/empty/error/success states, and don't add client-side
  validation as a substitute for server-side validation.

## Architecture policy

Follow the architecture already present. Do not introduce layers merely because they're common —
in particular, do not automatically create repositories, services, use cases, controllers,
gateways, adapters, domain events, command/query buses, or generic CRUD frameworks. If work
genuinely requires a new architectural decision: present the smallest viable option, state its
immediate trade-off, wait for approval, then implement only the approved part. Do not move
business, persistence, or UI logic between layers as part of an unrelated task.
