CREATE TABLE IF NOT EXISTS klines (
    source TEXT NOT NULL,
    symbol TEXT NOT NULL,
    interval TEXT NOT NULL,
    open_time TIMESTAMPTZ NOT NULL,
    open_price NUMERIC NOT NULL,
    high_price NUMERIC NOT NULL,
    low_price NUMERIC NOT NULL,
    close_price NUMERIC NOT NULL,
    base_volume NUMERIC NOT NULL,
    quote_volume NUMERIC NOT NULL,
    source_count INTEGER NOT NULL,
    is_complete BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT klines_source_not_empty CHECK (source <> ''),
    CONSTRAINT klines_symbol_not_empty CHECK (symbol <> ''),
    CONSTRAINT klines_interval_not_empty CHECK (interval <> ''),
    CONSTRAINT klines_source_count_positive CHECK (source_count > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS klines_unique
    ON klines (source, symbol, interval, open_time);

CREATE INDEX IF NOT EXISTS klines_lookup_idx
    ON klines (source, symbol, interval, open_time DESC);
