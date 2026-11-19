SELECT id,
       name,
       client_id,
       client_secret,
       redirect_uri,
       active,
       client_kind,
       app_id,
       app_version,
       user_agent,
       last_used_at,
       created_at,
       updated_at
FROM oauth_apps
WHERE id = $1
