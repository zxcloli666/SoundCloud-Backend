WITH roots AS (
    SELECT media.id, media.owner_user, media.status AS was
    FROM chat.media AS media
    JOIN chat.conversations AS conversation
      ON conversation.id = media.origin_conversation
    WHERE media.origin_conversation = $1
      AND media.kind <> 'thumb'
      AND media.status IN ('reserved', 'uploaded')
      AND conversation.kind = 'room'
      AND conversation.deleted_at IS NOT NULL
    ORDER BY media.id
    FOR UPDATE OF media
),
moved AS (
    UPDATE chat.media AS media
    SET status = 'deleting',
        delete_after = NULL,
        lease_until = NULL,
        restorable_until = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = roots.was
    RETURNING media.bytes, roots.id AS root, roots.owner_user, roots.was
),
released AS (
    SELECT owner_user,
           coalesce(sum(bytes) FILTER (WHERE was = 'reserved'), 0)::bigint AS reserved,
           coalesce(sum(bytes) FILTER (WHERE was = 'uploaded'), 0)::bigint AS bytes,
           count(DISTINCT root) FILTER (WHERE was = 'uploaded')::integer AS files
    FROM moved
    GROUP BY owner_user
),
charged AS (
    UPDATE chat.usage AS usage
    SET reserved = greatest(usage.reserved - released.reserved, 0),
        bytes = greatest(usage.bytes - released.bytes, 0),
        files = greatest(usage.files - released.files, 0),
        updated_at = now()
    FROM released
    WHERE usage.user_id = released.owner_user
    RETURNING usage.user_id
)
SELECT count(DISTINCT root)::bigint AS "ended!"
FROM moved
