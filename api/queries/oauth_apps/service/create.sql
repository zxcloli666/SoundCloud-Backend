INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active, client_kind, app_id, app_version, user_agent)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
RETURNING id, name, client_id, client_secret, redirect_uri, active, client_kind, app_id, app_version, user_agent,
          last_used_at, created_at, updated_at
