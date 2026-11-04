SELECT user_id,
       dm_policy
FROM chat.users
WHERE user_id = ANY ($1)
