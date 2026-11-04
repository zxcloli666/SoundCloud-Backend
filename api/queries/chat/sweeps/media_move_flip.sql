UPDATE chat.media
SET store = $3
WHERE (id = $1 OR parent_id = $1)
  AND store = $2
