WITH owned AS (
    SELECT oauth_app_id
    FROM oauth_app_token_refresh_state
    WHERE oauth_app_id = $1
      AND lease_id = $2
    FOR UPDATE
), recorded AS (
    INSERT INTO oauth_app_tokens (
        oauth_app_id,
        access_token,
        expires_at,
        refreshed_at,
        refresh_attempts,
        last_refresh_error
    )
    SELECT oauth_app_id, NULL, now(), now(), 1, $4
    FROM owned
    ON CONFLICT (oauth_app_id) DO UPDATE
    SET refreshed_at = EXCLUDED.refreshed_at,
        refresh_attempts = oauth_app_tokens.refresh_attempts + 1,
        last_refresh_error = EXCLUDED.last_refresh_error
), released AS (
    UPDATE oauth_app_token_refresh_state AS state
    SET retry_at = $3,
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
