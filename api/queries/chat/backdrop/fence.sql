UPDATE chat.conversations
SET backdrop_fence = $2
WHERE id = $1
  AND kind = 'room'
  AND deleted_at IS NULL
RETURNING id
