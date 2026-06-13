-- Audit database schema. Applied idempotently at service startup.
--
-- Two kinds of tables:
--   1. Chain-derived (address_book, audit_records): pure reconstructions of
--      on-chain data. Safe to TRUNCATE at any time; the indexer rebuilds them
--      from the blockchain + viewing key ("Rebuild from Chain").
--   2. Operational bookkeeping (workers, submissions): gateway-side state for
--      user_index allocation and the pending->confirmed lifecycle.

CREATE TABLE IF NOT EXISTS workers (
    user_index   INTEGER PRIMARY KEY,          -- ZIP 32 account index under the org seed
    name         TEXT NOT NULL,
    role         TEXT NOT NULL,
    address      TEXT NOT NULL,                -- derived unified address
    enroll_txid  TEXT,                         -- enrollment broadcast txid
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS submissions (
    id           UUID PRIMARY KEY,
    user_index   INTEGER NOT NULL,
    kind         TEXT NOT NULL,                -- 'enroll' | 'event'
    payload      JSONB NOT NULL,               -- the Record as JSON (pre-chain view)
    status       TEXT NOT NULL DEFAULT 'pending', -- pending -> broadcast -> confirmed
    txid         TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Chain-derived: enrollment memos.
CREATE TABLE IF NOT EXISTS address_book (
    address      TEXT PRIMARY KEY,
    user_index   INTEGER,
    name         TEXT NOT NULL,
    role         TEXT NOT NULL,
    active       BOOLEAN NOT NULL DEFAULT TRUE,
    txid         TEXT NOT NULL,
    block_height BIGINT NOT NULL,
    block_time   TIMESTAMPTZ
);

-- Chain-derived: event memos.
CREATE TABLE IF NOT EXISTS audit_records (
    txid         TEXT NOT NULL,
    output_pool  TEXT NOT NULL,                -- 'sapling' | 'orchard'
    output_index INTEGER NOT NULL,
    block_height BIGINT NOT NULL,
    block_time   TIMESTAMPTZ,
    address      TEXT,                         -- receiving address (identity)
    user_index   INTEGER,
    item_id      TEXT NOT NULL,
    event_type   TEXT NOT NULL,
    quantity     BIGINT NOT NULL,
    temp_centi   INTEGER NOT NULL,
    client_ts    TIMESTAMPTZ NOT NULL,
    notes        TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (txid, output_pool, output_index)
);

CREATE INDEX IF NOT EXISTS audit_records_item_idx ON audit_records (item_id);
CREATE INDEX IF NOT EXISTS audit_records_height_idx ON audit_records (block_height);
