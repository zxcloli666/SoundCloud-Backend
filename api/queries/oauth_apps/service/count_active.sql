SELECT COUNT(*) ::int8 AS "count!"
FROM oauth_apps
WHERE active = true
  AND client_kind = 'default'
