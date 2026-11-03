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
WHERE connection.id = $1
