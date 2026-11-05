WITH wanted AS (
    SELECT DISTINCT wanted.media_id, wanted.seq
    FROM unnest($2::uuid[], $3::int8[]) AS wanted (media_id, seq)
),
usable AS (
    SELECT wanted.media_id, wanted.seq, media.status, media.owner_user
    FROM wanted
    JOIN chat.media AS media ON media.id = wanted.media_id
    JOIN chat.conversations AS conversation ON conversation.id = $1
    WHERE media.kind = 'image'
      AND media.status IN ('uploaded', 'attached', 'held')
      AND conversation.deleted_at IS NULL
      AND (
          media.origin_conversation = $1
          OR (
              conversation.kind <> 'room'
              AND NOT EXISTS (
                  SELECT 1
                  FROM chat.conversations AS origin
                  WHERE origin.id = media.origin_conversation
                    AND origin.kind = 'room'
              )
          )
      )
),
referenced AS (
    INSERT INTO chat.media_refs (media_id, conversation_id, seq)
    SELECT media_id, $1, seq
    FROM usable
    ON CONFLICT DO NOTHING
),
roots AS (
    SELECT DISTINCT media_id AS id, status AS was, owner_user
    FROM usable
    WHERE status IN ('uploaded', 'held')
),
moved AS (
    UPDATE chat.media AS media
    SET status = 'attached',
        delete_after = NULL,
        restorable_until = NULL
    FROM roots
    WHERE (media.id = roots.id OR media.parent_id = roots.id)
      AND media.status = roots.was
    RETURNING media.bytes, roots.id AS root, roots.owner_user, roots.was
),
charged AS (
    SELECT owner_user,
           sum(bytes)::bigint AS bytes,
           count(DISTINCT root)::integer AS files
    FROM moved
    WHERE was = 'held'
    GROUP BY owner_user
),
usage AS (
    UPDATE chat.usage AS usage
    SET bytes = usage.bytes + charged.bytes,
        files = usage.files + charged.files,
        updated_at = now()
    FROM charged
    WHERE usage.user_id = charged.owner_user
)
SELECT DISTINCT media_id AS "media_id!"
FROM usable
ORDER BY 1
