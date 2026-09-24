WITH expired AS (
    SELECT entry.cache_key
    FROM ai_resolver_cache AS entry
    WHERE entry.expires_at < now()
    ORDER BY entry.expires_at
    FOR UPDATE OF entry SKIP LOCKED
    LIMIT 10_000
)
DELETE FROM ai_resolver_cache AS entry
USING expired
WHERE entry.cache_key = expired.cache_key
