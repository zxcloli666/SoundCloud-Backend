SELECT oauth_app_id,
       device_id,
       access_token,
       expires_at,
       status
FROM mobile_credentials
WHERE soundcloud_user_id = $1
