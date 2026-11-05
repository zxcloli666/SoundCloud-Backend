SELECT media.id, media.object_key, media.status
FROM chat.media AS media
WHERE media.store = $1
  AND media.object_key > $2
  AND media.kind <> 'thumb'
  AND media.status IN ('uploaded', 'attached', 'held', 'lost')
ORDER BY media.object_key
LIMIT $3
