-- Drop `accounts.initial_balance`: an opening balance will be recorded as a
-- transaction instead, so an account carries no monetary amount of its own.
ALTER TABLE accounts
DROP COLUMN initial_balance;
