-- Record each transaction's amount in its account's default currency, translated
-- once at the FX rate on its booking date. `account_amount` is that value;
-- `fx_rate` / `fx_rate_date` are the rate used and the reference date it came
-- from. All three are NULL for a foreign-currency transaction whose rate has not
-- been fetched yet ("conversion pending"). `account_amount` equals `amount`
-- exactly when the transaction is already in the account's currency (then
-- `fx_rate` / `fx_rate_date` stay NULL).

ALTER TABLE transactions
ADD COLUMN account_amount NUMERIC(38, 18) DEFAULT NULL,
ADD COLUMN fx_rate NUMERIC(38, 18) DEFAULT NULL,
ADD COLUMN fx_rate_date DATE DEFAULT NULL;

-- A value date before the booking date was never meaningful (a value date is
-- when funds settle, on or after the transaction). Any such legacy row held bad
-- data; clear the unreliable value date rather than invent one, then enforce it.
UPDATE transactions
SET value_date = NULL
WHERE value_date < booking_date;

ALTER TABLE transactions
ADD CONSTRAINT transactions_value_not_before_booking
CHECK (value_date IS NULL OR value_date >= booking_date);

ALTER TABLE transactions
ADD CONSTRAINT transactions_fx_rate_positive
CHECK (fx_rate IS NULL OR fx_rate > 0);

ALTER TABLE transactions
ADD CONSTRAINT transactions_fx_pair_consistent
CHECK ((fx_rate IS NULL) = (fx_rate_date IS NULL));

-- Same-currency transactions need no conversion: their account amount is the
-- amount itself. Foreign-currency rows stay NULL until the app backfills them.
UPDATE transactions AS t
SET account_amount = t.amount
FROM accounts AS a
WHERE a.id = t.account_id AND a.default_asset_id = t.asset_id;

-- Replace the per-(account, currency) view with one row per account, valued in
-- the account's own currency: the signed sum of every transaction's account
-- amount. Transactions still pending conversion are excluded; the app fills them
-- shortly after startup.
DROP VIEW account_balances;

CREATE VIEW account_balances AS
SELECT
    acc.user_id,
    t.account_id,
    acc.default_asset_id AS asset_id,
    sum(t.account_amount) AS balance
FROM transactions AS t
INNER JOIN accounts AS acc ON t.account_id = acc.id
WHERE
    t.deleted_at IS NULL
    AND acc.deleted_at IS NULL
    AND t.account_amount IS NOT NULL
GROUP BY acc.user_id, t.account_id, acc.default_asset_id;
