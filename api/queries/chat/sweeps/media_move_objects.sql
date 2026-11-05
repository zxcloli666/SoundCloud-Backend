SELECT id, object_key
FROM chat.media
WHERE (id = $1 OR parent_id = $1)
  AND store = $2
ORDER BY kind = 'thumb', id
