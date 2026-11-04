SELECT id
FROM chat.conversations
WHERE kind = 'room'
  AND deleted_at IS NULL
  AND room_alive_until < now()
ORDER BY room_alive_until
LIMIT $1
