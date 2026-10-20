UPDATE chat.conversations
SET room_alive_until = now() + interval '10 minutes'
WHERE id = $1
  AND kind = 'room'
  AND deleted_at IS NULL
