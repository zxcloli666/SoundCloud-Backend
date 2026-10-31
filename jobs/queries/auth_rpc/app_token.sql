SELECT token.generation,
       token.access_token AS "access_token!",
       token.expires_at
FROM oauth_app_tokens AS token
JOIN oauth_apps AS app ON app.id = token.oauth_app_id
WHERE token.oauth_app_id = $1
  AND token.access_token IS NOT NULL
  AND token.expires_at > now()
  AND app.active
