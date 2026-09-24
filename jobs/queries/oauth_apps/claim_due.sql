WITH candidates AS (
    SELECT app.id
    FROM oauth_apps AS app
    LEFT JOIN oauth_app_tokens AS token ON token.oauth_app_id = app.id
    LEFT JOIN oauth_app_token_refresh_state AS state ON state.oauth_app_id = app.id
    WHERE app.active
      AND (token.oauth_app_id IS NULL OR token.expires_at <= now() + interval '5 minutes')
      AND COALESCE(state.retry_at, '-infinity'::timestamptz) <= now()
      AND (state.lease_expires_at IS NULL OR state.lease_expires_at <= now())
    ORDER BY token.expires_at NULLS FIRST, app.id
    FOR UPDATE OF app SKIP LOCKED
    LIMIT $1
), claimed AS (
    INSERT INTO oauth_app_token_refresh_state (
        oauth_app_id,
        retry_at,
        lease_id,
        lease_expires_at,
        updated_at
    )
    SELECT id,
           now(),
           gen_random_uuid(),
           now() + $2::bigint * interval '1 millisecond',
           now()
    FROM candidates
    ON CONFLICT (oauth_app_id) DO UPDATE
    SET lease_id = EXCLUDED.lease_id,
        lease_expires_at = EXCLUDED.lease_expires_at,
        updated_at = EXCLUDED.updated_at
    WHERE oauth_app_token_refresh_state.retry_at <= now()
      AND (
          oauth_app_token_refresh_state.lease_expires_at IS NULL
          OR oauth_app_token_refresh_state.lease_expires_at <= now()
      )
    RETURNING oauth_app_id, lease_id
)
SELECT app.id,
       app.client_id,
       app.client_secret,
       token.refresh_token AS "refresh_token?",
       token.refresh_attempts AS "refresh_attempts?",
       claimed.lease_id AS "lease_id!"
FROM claimed
JOIN oauth_apps AS app ON app.id = claimed.oauth_app_id
LEFT JOIN oauth_app_tokens AS token ON token.oauth_app_id = app.id
