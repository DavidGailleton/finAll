-- `account_balances`: the exact per-currency balance of each account, as the
-- signed sum of its non-deleted transactions. No date filter (future-dated
-- transactions are included), no currency conversion, no rates. Transfer legs
-- are ordinary `transactions` rows and are counted here like any other. A row
-- exists only for an (account, asset) pair with at least one non-deleted
-- transaction.
CREATE VIEW account_balances AS
SELECT
    acc.user_id,
    t.account_id,
    t.asset_id,
    sum(t.amount) AS balance
FROM transactions AS t
INNER JOIN accounts AS acc ON t.account_id = acc.id
WHERE
    t.deleted_at IS NULL
    AND acc.deleted_at IS NULL
GROUP BY acc.user_id, t.account_id, t.asset_id;
