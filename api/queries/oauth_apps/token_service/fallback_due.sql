SELECT app.id,
       app.client_id,
       app.client_secret,
       app.redirect_uri
FROM oauth_apps AS app
WHERE app.active
  AND ($2::uuid IS NULL OR app.id = $2)
  AND NOT EXISTS (
      SELECT 1
      FROM oauth_app_tokens AS token
      WHERE token.oauth_app_id = app.id
        AND token.access_token IS NOT NULL
        AND token.expires_at > $1
  )
  AND NOT EXISTS (
      SELECT 1
      FROM oauth_app_tokens_local AS token
      WHERE token.oauth_app_id = app.id
        AND token.expires_at > $1
  )
ORDER BY app.id
