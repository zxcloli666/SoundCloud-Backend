INSERT INTO mobile_credentials (
    soundcloud_user_id,
    oauth_app_id,
    device_id,
    access_token,
    refresh_token,
    expires_at,
    scope,
    linked_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, now())
