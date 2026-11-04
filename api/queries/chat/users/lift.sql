UPDATE chat.users
SET restricted_until = NULL,
    restriction_scope = '{}',
    updated_at = now()
WHERE user_id = $1
