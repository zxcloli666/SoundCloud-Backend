SELECT media.id
FROM chat.media AS media
WHERE media.status = 'deleting'
  AND media.kind <> 'thumb'
  AND (media.restorable_until IS NULL OR media.restorable_until <= now())
  AND (
      EXISTS (
          SELECT 1
          FROM chat.media_refs AS ref
          WHERE ref.media_id = media.id
      )
      OR EXISTS (
          SELECT 1
          FROM chat.media_holds AS hold
          WHERE hold.media_id = media.id
            AND hold.until > now()
      )
  )
ORDER BY media.id
LIMIT $1
FOR UPDATE OF media SKIP LOCKED
