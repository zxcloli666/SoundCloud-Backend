SELECT user_id, read_receipts, typing, presence_share
FROM chat.users
WHERE user_id = ANY($1)
