WITH wanted AS (
    SELECT DISTINCT wanted.media_id, wanted.seq
    FROM unnest($3::uuid[], $4::int8[]) AS wanted (media_id, seq)
),
usable AS (
    SELECT wanted.media_id, wanted.seq
    FROM wanted
    JOIN chat.media AS media ON media.id = wanted.media_id
    JOIN chat.conversations AS conversation ON conversation.id = $1
    WHERE media.kind = 'image'
      AND media.status IN ('uploaded', 'attached')
      AND media.owner_user = $2
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
attached AS (
    UPDATE chat.media AS media
    SET status = 'attached',
        delete_after = NULL
    FROM (SELECT DISTINCT media_id FROM usable) AS root
    WHERE (media.id = root.media_id OR media.parent_id = root.media_id)
      AND media.status = 'uploaded'
)
SELECT DISTINCT media_id AS "media_id!"
FROM usable
ORDER BY 1
