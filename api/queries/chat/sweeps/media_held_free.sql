SELECT media.id
FROM chat.media AS media
WHERE media.status = 'held'
  AND media.kind <> 'thumb'
  AND NOT EXISTS (
      SELECT 1
      FROM chat.media_holds AS hold
      WHERE hold.media_id = media.id
        AND hold.until > now()
  )
ORDER BY media.id
LIMIT $1
