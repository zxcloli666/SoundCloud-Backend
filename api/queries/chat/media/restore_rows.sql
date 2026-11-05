WITH roots AS MATERIALIZED (
    SELECT media.id,
           EXISTS (
               SELECT 1
               FROM chat.media_refs AS ref
               WHERE ref.media_id = media.id
           ) AS referenced
    FROM chat.media AS media
    WHERE media.id = ANY($2::uuid[])
      AND media.owner_user = $1
      AND media.kind <> 'thumb'
      AND media.status = 'deleting'
      AND media.restorable_until > now()
      AND media.lease_until IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM chat.media AS part
          WHERE part.parent_id = media.id
            AND (part.status <> 'deleting'
                 OR part.lease_until IS NOT NULL
                 OR part.restorable_until IS NULL
                 OR part.restorable_until <= now())
      )
),
sized AS (
    SELECT coalesce(sum(media.bytes), 0)::bigint AS bytes,
           (SELECT count(*) FROM roots)::integer AS files
    FROM chat.media AS media
    JOIN roots ON media.id = roots.id OR media.parent_id = roots.id
    WHERE media.status = 'deleting'
      AND media.restorable_until > now()
),
charged AS (
    UPDATE chat.usage AS usage
    SET bytes = usage.bytes + sized.bytes,
        files = usage.files + sized.files,
        updated_at = now()
    FROM sized
    WHERE usage.user_id = $1
      AND sized.files > 0
      AND usage.bytes + usage.reserved + sized.bytes <= $3::bigint
    RETURNING usage.bytes, usage.reserved
),
moved AS (
    UPDATE chat.media AS media
    SET status = CASE WHEN roots.referenced THEN 'attached' ELSE 'uploaded' END,
        delete_after = CASE WHEN roots.referenced THEN NULL ELSE now() + $4::int * interval '1 second' END,
        restorable_until = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = 'deleting'
      AND media.restorable_until > now()
      AND EXISTS (SELECT 1 FROM charged)
    RETURNING media.id, roots.id AS root
)
SELECT sized.files AS "files!",
       (SELECT count(DISTINCT root) FROM moved)::integer AS "restored!",
       charged.bytes AS "bytes?",
       charged.reserved AS "reserved?"
FROM sized
LEFT JOIN charged ON true
