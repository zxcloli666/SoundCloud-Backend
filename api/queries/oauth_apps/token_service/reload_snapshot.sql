SELECT token.oauth_app_id,
       token.generation,
       token.access_token AS "access_token!",
       token.expires_at
FROM oauth_app_tokens AS token
JOIN oauth_apps AS app ON app.id = token.oauth_app_id
WHERE token.expires_at > $1
  AND token.access_token IS NOT NULL
  AND app.active = true
