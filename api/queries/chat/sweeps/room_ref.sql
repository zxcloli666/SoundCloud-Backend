SELECT room_ref
FROM chat.conversations
WHERE id = $1
  AND kind = 'room'
