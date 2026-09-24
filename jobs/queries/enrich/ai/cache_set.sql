INSERT INTO ai_resolver_cache (cache_key, reply, expires_at)
VALUES ($1, $2, now() + ($3::bigint * interval '1 second'))
ON CONFLICT (cache_key) DO UPDATE
    SET reply      = excluded.reply,
        expires_at = excluded.expires_at
