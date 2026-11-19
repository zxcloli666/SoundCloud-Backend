UPDATE oauth_apps
SET name          = COALESCE($2, name),
    client_secret = COALESCE($3, client_secret),
    redirect_uri  = COALESCE($4, redirect_uri),
    active        = COALESCE($5, active),
    app_id        = COALESCE($6, app_id),
    app_version   = COALESCE($7, app_version),
    user_agent    = COALESCE($8, user_agent),
    updated_at    = now()
WHERE id = $1
RETURNING id, name, client_id, client_secret, redirect_uri, active, client_kind, app_id, app_version, user_agent,
          last_used_at, created_at, updated_at
