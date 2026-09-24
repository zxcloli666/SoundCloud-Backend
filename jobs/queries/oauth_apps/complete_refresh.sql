WITH owned AS (
    SELECT oauth_app_id
    FROM oauth_app_token_refresh_state
    WHERE oauth_app_id = $1
      AND lease_id = $2
    FOR UPDATE
), stored AS (
    INSERT INTO oauth_app_tokens (
        oauth_app_id,
        access_token,
        refresh_token,
        scope,
        expires_at,
        refreshed_at,
        refresh_attempts,
        last_refresh_error
    )
    SELECT oauth_app_id,
           $3,
           $4,
           $5,
           now() + $6::bigint * interval '1 second',
           now(),
           0,
           NULL
    FROM owned
    ON CONFLICT (oauth_app_id) DO UPDATE
    SET access_token = EXCLUDED.access_token,
        generation = gen_random_uuid(),
        refresh_token = EXCLUDED.refresh_token,
        scope = EXCLUDED.scope,
        expires_at = EXCLUDED.expires_at,
        refreshed_at = EXCLUDED.refreshed_at,
        refresh_attempts = 0,
        last_refresh_error = NULL
), released AS (
    UPDATE oauth_app_token_refresh_state AS state
    SET retry_at = now(),
        lease_id = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM owned
    WHERE state.oauth_app_id = owned.oauth_app_id
      AND state.lease_id = $2
    RETURNING state.oauth_app_id
)
SELECT oauth_app_id
FROM released
