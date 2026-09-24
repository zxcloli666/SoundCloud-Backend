CREATE TABLE IF NOT EXISTS ai_resolver_cache
(
    cache_key  text        PRIMARY KEY,
    reply      jsonb       NOT NULL,
    expires_at timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS ai_resolver_cache_expiry_idx
    ON ai_resolver_cache (expires_at);

CREATE TABLE IF NOT EXISTS ai_resolver_budget
(
    spent_on date   PRIMARY KEY,
    spent    bigint NOT NULL DEFAULT 0
);
