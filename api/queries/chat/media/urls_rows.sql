SELECT media.id,
       media.store,
       media.object_key,
       media.status,
       media.owner_user = $2 AS "own!",
       ($3::uuid IS NOT NULL AND EXISTS (
           SELECT 1
           FROM chat.media_refs AS ref
           WHERE ref.media_id = coalesce(media.parent_id, media.id)
             AND ref.conversation_id = $3::uuid
             AND (ref.seq = 0 OR ref.seq > $4::bigint)
       )) AS "shared!"
FROM chat.media AS media
WHERE media.id = ANY($1::uuid[])
