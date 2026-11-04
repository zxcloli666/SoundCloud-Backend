UPDATE chat.media AS media
SET status = CASE
        WHEN EXISTS (
            SELECT 1
            FROM chat.media_holds AS hold
            WHERE hold.media_id = coalesce(media.parent_id, media.id)
              AND hold.until > now()
        ) THEN 'held'
        ELSE 'deleting'
    END,
    restorable_until = NULL,
    lease_until = NULL
WHERE (media.id = ANY($1::uuid[]) OR media.parent_id = ANY($1::uuid[]))
  AND media.status = 'deleting'
  AND (media.restorable_until IS NULL OR media.restorable_until <= now())
RETURNING media.id
