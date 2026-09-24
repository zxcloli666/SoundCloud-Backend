SELECT connection.id,
       connection.soundcloud_user_id,
       connection.oauth_app_id,
       connection.access_token,
       connection.refresh_token,
       connection.expires_at,
       connection.scope,
       connection.refresh_generation,
       connection.refresh_failure_count,
       connection.refresh_lease_id,
       connection.refresh_lease_expires_at,
       connection.last_refresh_attempt_at,
       connection.last_refresh_success_at,
       connection.last_refresh_error_kind,
       connection.last_refresh_error,
       connection.retry_at
FROM sessions AS session
JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
WHERE session.id = $1
