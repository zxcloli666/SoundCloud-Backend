INSERT INTO chat.users (user_id)
SELECT unnest($1::text[])
ON CONFLICT (user_id) DO NOTHING
