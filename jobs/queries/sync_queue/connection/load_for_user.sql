SELECT connection.id,
       connection.soundcloud_user_id,
       connection.access_token,
       connection.refresh_token,
       connection.expires_at,
       connection.scope,
       connection.refresh_generation,
       connection.refresh_failure_count,
       connection.refresh_lease_id,
       connection.refresh_lease_expires_at,
       connection.last_refresh_error_kind,
       connection.retry_at,
       connection.oauth_app_id,
       app.client_id,
       app.client_secret,
       app.active AS "app_active?"
FROM soundcloud_connections AS connection
LEFT JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
WHERE connection.soundcloud_user_id = $1
ORDER BY (
             connection.expires_at > now()
             AND connection.last_refresh_error_kind IS DISTINCT FROM 'token_rejected'
             AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
         ) DESC,
         (app.id IS NOT NULL AND app.active) DESC,
         (connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required') DESC,
         (connection.retry_at IS NULL OR connection.retry_at <= now()) DESC,
         connection.updated_at DESC,
         connection.id
LIMIT 1
