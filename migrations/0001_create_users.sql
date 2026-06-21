CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    openid TEXT,
    unionid TEXT,
    country_code TEXT,
    pure_phone_number TEXT,
    phone_number TEXT,
    phone_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS users_openid_unique
    ON users (openid)
    WHERE openid IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS users_phone_unique
    ON users (country_code, pure_phone_number)
    WHERE country_code IS NOT NULL
      AND pure_phone_number IS NOT NULL;
