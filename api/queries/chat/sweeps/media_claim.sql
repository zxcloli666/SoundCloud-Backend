WITH picked AS (
    SELECT media.id
    FROM chat.media AS media
    WHERE media.status = 'deleting'
      AND media.kind <> 'thumb'
      AND (media.lease_until IS NULL OR media.lease_until <= now())
      AND (media.restorable_until IS NULL OR media.restorable_until <= now())
      AND NOT EXISTS (
          SELECT 1
          FROM chat.media_refs AS ref
          WHERE ref.media_id = media.id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM chat.media_holds AS hold
          WHERE hold.media_id = media.id
            AND hold.until > now()
      )
    ORDER BY media.lease_until NULLS FIRST
    LIMIT $1
    FOR UPDATE SKIP LOCKED
),
claimed AS (
    UPDATE chat.media AS media
    SET lease_until = now() + interval '5 minutes',
        attempts = least(media.attempts + 1, 32767),
        restorable_until = NULL
    FROM picked
    WHERE (media.id = picked.id OR media.parent_id = picked.id)
      AND media.status = 'deleting'
    RETURNING picked.id AS root, media.store, media.object_key
)
SELECT root AS "root!", store AS "store!", object_key AS "object_key!"
FROM claimed
