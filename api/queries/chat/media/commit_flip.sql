WITH kept AS (
    SELECT kept.id, kept.object_key
    FROM unnest($4::uuid[], $5::text[]) AS kept(id, object_key)
),
moved AS (
    UPDATE chat.media AS media
    SET status = 'uploaded',
        object_key = kept.object_key,
        delete_after = now() + $3::int * interval '1 second'
    FROM kept
    WHERE media.id = kept.id
      AND (media.id = $1 OR media.parent_id = $1)
      AND media.owner_user = $2
      AND media.status = 'reserved'
      AND EXISTS (
          SELECT 1
          FROM chat.media AS root
          WHERE root.id = $1
            AND root.status = 'reserved'
            AND root.delete_after > now()
      )
    RETURNING media.id, media.bytes
),
totals AS (
    SELECT coalesce(sum(bytes), 0)::bigint AS bytes,
           count(*) FILTER (WHERE id = $1)::integer AS files
    FROM moved
),
charged AS (
    UPDATE chat.usage AS usage
    SET reserved = greatest(usage.reserved - totals.bytes, 0),
        bytes = usage.bytes + totals.bytes,
        files = usage.files + totals.files,
        updated_at = now()
    FROM totals
    WHERE usage.user_id = $2
      AND totals.files > 0
    RETURNING usage.bytes, usage.reserved
)
SELECT totals.files AS "files!",
       charged.bytes AS "bytes?",
       charged.reserved AS "reserved?"
FROM totals
LEFT JOIN charged ON true
