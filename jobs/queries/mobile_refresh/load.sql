SELECT credential.soundcloud_user_id,
       credential.oauth_app_id,
       credential.device_id,
       credential.access_token,
       credential.refresh_token,
       credential.expires_at,
       credential.status,
       credential.refresh_generation,
       credential.refresh_failure_count,
       credential.refresh_lease_id,
       credential.refresh_lease_expires_at,
       credential.retry_at,
       credential.last_refresh_error_kind,
       app.client_id,
       app.client_secret,
       app.app_id,
       app.app_version,
       app.user_agent,
       (app.active AND app.client_kind = 'mobile') AS "app_usable!"
FROM soundcloud_connections AS connection
JOIN mobile_credentials AS credential
    ON credential.soundcloud_user_id = connection.soundcloud_user_id
JOIN oauth_apps AS app ON app.id = credential.oauth_app_id
WHERE connection.id = $1
