SELECT restricted_until AS "restricted_until!", restriction_scope
FROM chat.users
WHERE user_id = $1
  AND restricted_until > now()
