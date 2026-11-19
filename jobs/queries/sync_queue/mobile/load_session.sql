SELECT credential.soundcloud_user_id AS "account!",
       credential.oauth_app_id AS "oauth_app_id!",
       credential.device_id AS "device_id!",
       credential.access_token AS "access_token!",
       credential.expires_at AS "expires_at!",
       credential.status AS "status!",
       credential.plan,
       connection.id AS "connection_id!",
       app.client_id AS "client_id!",
       app.app_id,
       app.app_version,
       app.user_agent
FROM mobile_credentials AS credential
JOIN oauth_apps AS app
    ON app.id = credential.oauth_app_id
   AND app.active
   AND app.client_kind = 'mobile'
JOIN LATERAL (
    SELECT id
    FROM soundcloud_connections
    WHERE soundcloud_user_id = credential.soundcloud_user_id
    ORDER BY updated_at DESC
    LIMIT 1
) AS connection ON true
WHERE credential.soundcloud_user_id = $1
