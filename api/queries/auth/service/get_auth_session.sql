SELECT session.id,
       session.soundcloud_connection_id,
       connection.soundcloud_user_id,
       connection.username,
       connection.oauth_app_id,
       app.active AS oauth_app_active,
       connection.expires_at,
       connection.refresh_lease_id,
       connection.refresh_lease_expires_at,
       connection.last_refresh_attempt_at,
       connection.last_refresh_success_at,
       connection.last_refresh_error_kind,
       connection.last_refresh_error,
       connection.retry_at
FROM sessions AS session
LEFT JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
LEFT JOIN oauth_apps AS app
    ON app.id = connection.oauth_app_id
WHERE session.id = $1
