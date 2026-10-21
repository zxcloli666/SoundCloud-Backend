SELECT dm_policy, read_receipts, typing, presence_share
FROM chat.users
WHERE user_id = $1
