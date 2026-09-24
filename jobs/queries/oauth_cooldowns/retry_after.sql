SELECT GREATEST(
           1,
           ceil(EXTRACT(EPOCH FROM (retry_at - now())))::bigint
       ) AS "retry_after_seconds!"
FROM oauth_app_request_cooldowns
WHERE oauth_app_id = $1
  AND retry_at > now()
