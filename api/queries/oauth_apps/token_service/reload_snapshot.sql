SELECT token.oauth_app_id AS "oauth_app_id!",
       token.generation AS "generation!",
       token.access_token AS "access_token!",
       token.expires_at AS "expires_at!"
FROM (
    SELECT oauth_app_id, generation, access_token, expires_at
    FROM oauth_app_tokens
    UNION ALL
    SELECT oauth_app_id, generation, access_token, expires_at
    FROM oauth_app_tokens_local
) AS token
JOIN oauth_apps AS app ON app.id = token.oauth_app_id
WHERE token.expires_at > $1
  AND token.access_token IS NOT NULL
  AND app.active = true
