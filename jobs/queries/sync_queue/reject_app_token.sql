WITH shared AS (
    UPDATE oauth_app_tokens
    SET expires_at = LEAST(expires_at, now())
    WHERE oauth_app_id = $1
      AND generation = $2
    RETURNING oauth_app_id
), local AS (
    UPDATE oauth_app_tokens_local
    SET expires_at = LEAST(expires_at, now())
    WHERE oauth_app_id = $1
      AND generation = $2
    RETURNING oauth_app_id
)
SELECT oauth_app_id AS "oauth_app_id!" FROM shared
UNION ALL
SELECT oauth_app_id AS "oauth_app_id!" FROM local
