SELECT id
FROM chat.conversations
WHERE kind = 'room'
  AND room_ref = $1
  AND deleted_at IS NULL
