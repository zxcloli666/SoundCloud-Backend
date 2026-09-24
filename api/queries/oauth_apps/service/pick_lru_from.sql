WITH picked AS (
    SELECT id
    FROM oauth_apps
    WHERE active = true
      AND id = ANY ($1)
    ORDER BY last_used_at ASC NULLS FIRST, created_at ASC
    FOR UPDATE SKIP LOCKED
    LIMIT 1
)
UPDATE oauth_apps AS app
SET last_used_at = now(),
    updated_at = now()
FROM picked
WHERE app.id = picked.id
RETURNING app.id,
          app.name,
          app.client_id,
          app.client_secret,
          app.redirect_uri,
          app.active,
          app.last_used_at,
          app.created_at,
          app.updated_at
