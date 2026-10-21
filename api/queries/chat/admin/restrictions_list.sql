SELECT user_id, restricted_until AS "restricted_until!", restriction_scope, updated_at
FROM chat.users
WHERE restricted_until > now()
ORDER BY restricted_until DESC
LIMIT $1
