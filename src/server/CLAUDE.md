# src/server/ — Financial domain rules & testing

Applies to everything under `src/server/` (all `ssr`-only, per `src/CLAUDE.md`'s architecture
section) — most directly `src/server/assets/` and `src/server/auth/`. See the root `CLAUDE.md`
for scope/workflow/security and `src/CLAUDE.md` for the general Rust/Leptos rules.

Financial correctness takes priority over convenience. **Never invent a financial rule.** If a
required rule is not already defined by the code or the request, ask the developer.

## Monetary values

The database stores financial amounts as `NUMERIC(38, 18)`.

- Never use `f32`/`f64` for money, and never convert a database decimal amount through floating
  point at any point in the pipeline. Use the project's established exact decimal representation.
  If none has been selected yet, or a decimal/money crate seems needed, ask before adding one.
- Keep amount and currency explicit together. Do not silently combine different currencies,
  convert currencies, round, or truncate values. Do not assume a currency's decimal precision or
  a rounding mode. Do not treat zero and missing data as equivalent. Preserve negative values
  when valid for the domain. Handle overflow and precision loss explicitly.
- Currency conversion requires an explicitly supplied exchange rate, source currency, target
  currency, valuation timestamp, and rounding policy — all four, not inferred.

## Transactions

Do not assume whether transaction amounts are signed or unsigned, or infer transaction direction,
status, category, or account effect from a name alone. Transfers between accounts must not
automatically be treated as income or expenses. Do not invent transaction categories,
reconciliation rules, duplicate-detection rules, pending-to-booked transition rules, balance
calculation rules, import matching behavior, or transaction reversal behavior. Rows from imports
or external providers are untrusted data.

## Accounts and balances

Do not assume the current balance is simply the sum of all transaction rows unless the existing
schema/domain rules explicitly establish that. Respect soft deletion (`deleted_at`), existing SQL
views, transaction status, currency boundaries, and database precision. Do not bypass the
`account_balances` view merely to duplicate its calculation in Rust unless explicitly requested,
and do not change the meaning of an existing balance calculation as part of an unrelated task.

## Loans

Do not invent interest rates, interest formulas, compounding periods, repayment schedules,
principal-versus-interest allocation, fees, penalties, or late-/early-repayment behavior. Loan
calculations require explicit business rules and focused tests.

## Investments

Keep these concepts separate: asset quantity, unit price, purchase cost, fees, cost basis, market
value, realized gain, unrealized gain, currency, and valuation timestamp. Do not assume a
cost-basis method, tax rule, current price, market-data source, or exchange rate.

## Cryptocurrency

Do not assume token precision, blockchain network, wallet ownership, confirmation rules, exchange
or custodian, price source, cost-basis method, or tax treatment. Use exact values for
cryptocurrency quantities — no floating-point arithmetic.

## Dates and time

Use the date/time types already selected by the project. Do not assume a timezone, use the
current date implicitly in a financial calculation, or make a test depend on the system clock.
Make reporting period boundaries explicit; distinguish booking date, value date, creation time,
and valuation time when relevant; do not assume every financial month or year follows calendar
boundaries.

## Testing rules

Add or modify tests only when requested or when a small focused test is necessary to verify the
exact financial behavior being implemented — do not generate a broad test suite for a local
change.

Relevant financial test cases: zero values; positive and negative values; maximum supported
precision; precision-loss boundaries; explicit rounding behavior; different currencies; missing
values; soft-deleted rows; overflow or range boundaries; date and timezone boundaries;
unauthorized access; duplicate submissions; transaction rollback behavior.

Tests must not depend on: the current system date, live market prices, live external APIs, real
credentials, real financial records, unstable network access, or test execution order.
