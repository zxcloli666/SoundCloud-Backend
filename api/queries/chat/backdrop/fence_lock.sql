SELECT id
FROM chat.conversations
WHERE id = $1
  AND backdrop_fence IS NOT DISTINCT FROM $2::uuid
FOR NO KEY UPDATE
