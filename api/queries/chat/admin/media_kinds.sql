SELECT id, kind
FROM chat.media
WHERE id = ANY($1::uuid[])
  AND kind IN ('avatar', 'backdrop')
