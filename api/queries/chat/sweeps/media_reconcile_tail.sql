SELECT object_key, coalesce(parent_id, id) AS "root!"
FROM chat.media
WHERE store = $1
  AND object_key > $2
  AND status IN ('attached', 'uploaded')
ORDER BY object_key
LIMIT $3
