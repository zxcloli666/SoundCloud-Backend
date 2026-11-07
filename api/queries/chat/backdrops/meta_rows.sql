SELECT media.id,
       media.kind,
       media.status,
       media.owner_user = $2 AS "own!",
       media.origin_conversation = $3 AS "origin_here!",
       EXISTS (
           SELECT 1
           FROM chat.media_refs AS ref
           WHERE ref.media_id = media.id
             AND ref.conversation_id = $3
             AND ref.seq = 0
       ) AS "current!"
FROM chat.media AS media
WHERE media.id = ANY($1::uuid[])
