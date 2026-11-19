SELECT id,
       client_id,
       client_secret,
       app_id AS "app_id!",
       app_version AS "app_version!",
       user_agent AS "user_agent!"
FROM oauth_apps
WHERE id = $1
  AND active
  AND client_kind = 'mobile'
  AND app_id IS NOT NULL
  AND app_version IS NOT NULL
  AND user_agent IS NOT NULL
