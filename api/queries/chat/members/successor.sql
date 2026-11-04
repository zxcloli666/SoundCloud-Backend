SELECT user_id
FROM chat.members
WHERE conversation_id = $1
  AND state = 'active'
  AND user_id <> $2
ORDER BY role = 'admin' DESC, joined_seq, user_id
LIMIT 1
