WITH roots AS MATERIALIZED (
    SELECT media.id,
           media.status AS was
    FROM chat.media AS media
    WHERE media.id = ANY($2::uuid[])
      AND media.owner_user = $1
      AND media.kind <> 'thumb'
      AND media.status IN ('reserved', 'uploaded', 'attached', 'lost')
),
moved AS (
    UPDATE chat.media AS media
    SET status = 'deleting',
        restorable_until = CASE
            WHEN roots.was IN ('uploaded', 'attached') THEN now() + $3::int * interval '1 second'
        END,
        lease_until = NULL,
        delete_after = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = roots.was
    RETURNING media.bytes, roots.id AS root, roots.was
),
released AS (
    SELECT coalesce(sum(bytes) FILTER (WHERE was IN ('uploaded', 'attached')), 0)::bigint AS bytes,
           coalesce(sum(bytes) FILTER (WHERE was = 'reserved'), 0)::bigint AS reserved,
           (count(DISTINCT root) FILTER (WHERE was IN ('uploaded', 'attached')))::integer AS files,
           (count(DISTINCT root))::integer AS roots
    FROM moved
),
charged AS (
    UPDATE chat.usage AS usage
    SET bytes = greatest(usage.bytes - released.bytes, 0),
        reserved = greatest(usage.reserved - released.reserved, 0),
        files = greatest(usage.files - released.files, 0),
        updated_at = now()
    FROM released
    WHERE usage.user_id = $1
      AND released.roots > 0
    RETURNING usage.bytes, usage.reserved
)
SELECT released.roots AS "deleted!",
       (released.bytes + released.reserved)::bigint AS "freed!",
       charged.bytes AS "bytes?",
       charged.reserved AS "reserved?"
FROM released
LEFT JOIN charged ON true
