SELECT COALESCE(max(substr(user_id, 4)::bigint), 0)::bigint AS "last!"
FROM chat.users
WHERE user_id LIKE 'lt:%'
  AND substr(user_id, 4) ~ '^[0-9]{1,18}$'
