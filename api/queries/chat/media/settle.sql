WITH roots AS (
    SELECT media.id,
           media.owner_user,
           media.status AS was,
           CASE
               WHEN media.status IN ('attached', 'held') AND EXISTS (
                   SELECT 1
                   FROM chat.media_holds AS hold
                   WHERE hold.media_id = media.id
                     AND hold.until > now()
               ) THEN 'held'
               ELSE 'deleting'
           END AS next
    FROM chat.media AS media
    WHERE media.id = ANY($1::uuid[])
      AND media.kind <> 'thumb'
      AND media.status IN ('attached', 'held', 'lost')
      AND NOT EXISTS (
          SELECT 1
          FROM chat.media_refs AS ref
          WHERE ref.media_id = media.id
      )
),
moved AS (
    UPDATE chat.media AS media
    SET status = roots.next,
        lease_until = NULL,
        delete_after = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = roots.was
      AND roots.next <> roots.was
    RETURNING media.bytes, roots.id AS root, roots.owner_user, roots.was
),
released AS (
    SELECT owner_user,
           sum(bytes)::bigint AS bytes,
           count(DISTINCT root)::integer AS files
    FROM moved
    WHERE was = 'attached'
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
SELECT count(DISTINCT root)::bigint AS "settled!"
FROM moved
