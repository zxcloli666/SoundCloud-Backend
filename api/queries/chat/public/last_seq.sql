SELECT last_seq
FROM chat.conversations
WHERE id = $1
  AND kind = 'public'
