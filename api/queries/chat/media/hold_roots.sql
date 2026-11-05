SELECT DISTINCT shown.id AS "id!"
FROM (
    SELECT ref.media_id AS id
    FROM chat.media_refs AS ref
    WHERE ref.conversation_id = $1
      AND ref.seq BETWEEN $2::bigint AND $3::bigint
      AND ref.seq > 0
    UNION
    SELECT media.id
    FROM chat.media AS media
    LEFT JOIN chat.conversations AS origin ON origin.id = media.origin_conversation
    WHERE media.id = ANY($4::uuid[])
      AND media.kind <> 'thumb'
      AND (
          media.origin_conversation = $1
          OR (media.kind = 'image' AND origin.kind IS DISTINCT FROM 'room')
      )
) AS shown
ORDER BY 1
