WITH roots AS (
    SELECT id, owner_user, status AS was
    FROM chat.media
    WHERE id = ANY($1::uuid[])
      AND store = $2
      AND kind <> 'thumb'
      AND status IN ('attached', 'uploaded')
    ORDER BY id
    FOR UPDATE
),
moved AS (
    UPDATE chat.media AS media
    SET status = 'lost',
        delete_after = NULL,
        lease_until = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = roots.was
    RETURNING media.bytes, roots.id AS root, roots.owner_user
),
released AS (
    SELECT owner_user,
           sum(bytes)::bigint AS bytes,
           count(DISTINCT root)::integer AS files
    FROM moved
    GROUP BY owner_user
),
charged AS (
    UPDATE chat.usage AS usage
    SET bytes = greatest(usage.bytes - released.bytes, 0),
        files = greatest(usage.files - released.files, 0),
        updated_at = now()
    FROM released
    WHERE usage.user_id = released.owner_user
    RETURNING usage.user_id
)
SELECT DISTINCT root AS "root!"
FROM moved
