CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    email TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT users_email_unique
    UNIQUE (email),

    CONSTRAINT users_email_not_empty
    CHECK (length(trim(email)) > 0),

    CONSTRAINT users_password_hash_not_empty
    CHECK (length(trim(password_hash)) > 0)
);

CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT sessions_user_id_fk
    FOREIGN KEY (user_id)
    REFERENCES users (id)
    ON DELETE CASCADE,

    CONSTRAINT sessions_token_hash_unique
    UNIQUE (token_hash),

    CONSTRAINT sessions_token_hash_not_empty
    CHECK (length(trim(token_hash)) > 0),

    CONSTRAINT sessions_expires_after_created
    CHECK (expires_at > created_at)
);

CREATE INDEX sessions_user_id_idx
ON sessions (user_id);

CREATE TABLE currencies (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    alphabetic_code TEXT NOT NULL,
    numeric_code TEXT,
    currency_name TEXT NOT NULL,
    symbol TEXT,
    minor_units SMALLINT NOT NULL DEFAULT 2,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT currencies_alphabetic_code_unique
    UNIQUE (alphabetic_code),

    CONSTRAINT currencies_numeric_code_unique
    UNIQUE (numeric_code),

    CONSTRAINT currencies_alphabetic_code_valid
    CHECK (alphabetic_code ~ '^[A-Z]{3}$'),

    CONSTRAINT currencies_numeric_code_valid
    CHECK (
        numeric_code IS NULL
        OR numeric_code ~ '^[0-9]{3}$'
    ),

    CONSTRAINT currencies_name_not_empty
    CHECK (length(trim(currency_name)) > 0),

    CONSTRAINT currencies_minor_units_valid
    CHECK (minor_units BETWEEN 0 AND 18),

    CONSTRAINT currencies_symbol_not_empty
    CHECK (
        symbol IS NULL
        OR length(trim(symbol)) > 0
    )
);

CREATE TABLE accounts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    default_currency_id UUID NOT NULL,
    account_name TEXT NOT NULL,
    account_type TEXT NOT NULL,
    initial_balance NUMERIC(38, 18) NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT accounts_user_id_fk
    FOREIGN KEY (user_id)
    REFERENCES users (id)
    ON DELETE CASCADE,

    CONSTRAINT accounts_default_currency_id_fk
    FOREIGN KEY (default_currency_id)
    REFERENCES currencies (id)
    ON DELETE RESTRICT,

    CONSTRAINT accounts_user_id_id_unique
    UNIQUE (user_id, id),

    CONSTRAINT accounts_name_not_empty
    CHECK (length(trim(account_name)) > 0),

    CONSTRAINT accounts_type_valid
    CHECK (
        account_type IN (
            'cash',
            'bank',
            'credit',
            'investment',
            'crypto',
            'loan',
            'other'
        )
    )
);

CREATE INDEX accounts_user_id_idx
ON accounts (user_id);

CREATE INDEX accounts_default_currency_id_idx
ON accounts (default_currency_id);

CREATE TABLE categories (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    category_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT categories_user_id_fk
    FOREIGN KEY (user_id)
    REFERENCES users (id)
    ON DELETE CASCADE,

    CONSTRAINT categories_user_id_id_unique
    UNIQUE (user_id, id),

    CONSTRAINT categories_user_id_name_unique
    UNIQUE (user_id, category_name),

    CONSTRAINT categories_name_not_empty
    CHECK (length(trim(category_name)) > 0),

    CONSTRAINT categories_kind_valid
    CHECK (kind IN ('income', 'expense'))
);

CREATE TABLE merchants (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    merchant_name TEXT NOT NULL,
    default_category_id UUID DEFAULT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT merchants_user_id_fk
    FOREIGN KEY (user_id)
    REFERENCES users (id)
    ON DELETE CASCADE,

    CONSTRAINT merchants_default_category_fk
    FOREIGN KEY (user_id, default_category_id)
    REFERENCES categories (user_id, id),

    CONSTRAINT merchants_user_id_id_unique
    UNIQUE (user_id, id),

    CONSTRAINT merchants_user_id_name_unique
    UNIQUE (user_id, merchant_name),

    CONSTRAINT merchants_name_not_empty
    CHECK (length(trim(merchant_name)) > 0)
);

CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    account_id UUID NOT NULL,
    currency_id UUID NOT NULL,
    category_id UUID DEFAULT NULL,
    merchant_id UUID DEFAULT NULL,
    amount NUMERIC(38, 18) NOT NULL,
    booking_date DATE NOT NULL,
    value_date DATE DEFAULT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT transactions_account_id_fk
    FOREIGN KEY (user_id, account_id)
    REFERENCES accounts (user_id, id)
    ON DELETE CASCADE,

    CONSTRAINT transactions_currency_id_fk
    FOREIGN KEY (currency_id)
    REFERENCES currencies (id)
    ON DELETE RESTRICT,

    CONSTRAINT transactions_category_id_fk
    FOREIGN KEY (user_id, category_id)
    REFERENCES categories (user_id, id),

    CONSTRAINT transactions_merchant_id_fk
    FOREIGN KEY (user_id, merchant_id)
    REFERENCES merchants (user_id, id),

    CONSTRAINT transactions_user_id_id_unique
    UNIQUE (user_id, id),

    CONSTRAINT transactions_amount_nonzero
    CHECK (amount <> 0)
);

CREATE INDEX transactions_account_id_idx
ON transactions (account_id);

CREATE INDEX transactions_currency_id_idx
ON transactions (currency_id);

CREATE INDEX transactions_user_id_booking_date_idx
ON transactions (user_id, booking_date);

CREATE INDEX transactions_created_at_idx
ON transactions (created_at);

CREATE TABLE transfers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL,
    source_transaction_id UUID NOT NULL,
    destination_transaction_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT transfers_user_id_fk
    FOREIGN KEY (user_id)
    REFERENCES users (id)
    ON DELETE CASCADE,

    CONSTRAINT transfers_source_transaction_fk
    FOREIGN KEY (user_id, source_transaction_id)
    REFERENCES transactions (user_id, id)
    ON DELETE CASCADE,

    CONSTRAINT transfers_destination_transaction_fk
    FOREIGN KEY (user_id, destination_transaction_id)
    REFERENCES transactions (user_id, id)
    ON DELETE CASCADE,

    CONSTRAINT transfers_source_unique
    UNIQUE (source_transaction_id),

    CONSTRAINT transfers_destination_unique
    UNIQUE (destination_transaction_id),

    CONSTRAINT transfers_distinct_transactions
    CHECK (source_transaction_id <> destination_transaction_id)
);

CREATE INDEX transfers_user_id_idx
ON transfers (user_id);

CREATE TABLE currency_rates (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    base_currency_id UUID NOT NULL,
    quote_currency_id UUID NOT NULL,
    rate NUMERIC(38, 18) NOT NULL,
    rate_source TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ DEFAULT NULL,

    CONSTRAINT currency_rates_base_currency_id_fk
    FOREIGN KEY (base_currency_id)
    REFERENCES currencies (id)
    ON DELETE RESTRICT,

    CONSTRAINT currency_rates_quote_currency_id_fk
    FOREIGN KEY (quote_currency_id)
    REFERENCES currencies (id)
    ON DELETE RESTRICT,

    CONSTRAINT currency_rates_distinct_currencies
    CHECK (base_currency_id <> quote_currency_id),

    CONSTRAINT currency_rates_rate_positive
    CHECK (rate > 0),

    CONSTRAINT currency_rates_rate_source_not_empty
    CHECK (length(trim(rate_source)) > 0),

    CONSTRAINT currency_rates_observation_unique
    UNIQUE (base_currency_id, quote_currency_id, rate_source, observed_at)
);
