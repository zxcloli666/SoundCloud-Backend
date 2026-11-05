SELECT root.id,
       root.kind,
       root.origin_conversation,
       thumb.id AS "thumb_id?",
       (root.bytes + coalesce(thumb.bytes, 0))::bigint AS "bytes!",
       root.status,
       (
           SELECT count(DISTINCT ref.conversation_id)
           FROM chat.media_refs AS ref
           WHERE ref.media_id = root.id
       )::integer AS "chats!",
       root.created_at
FROM chat.media AS root
LEFT JOIN chat.media AS thumb
    ON thumb.parent_id = root.id
   AND thumb.kind = 'thumb'
WHERE root.owner_user = $1
  AND root.kind <> 'thumb'
  AND root.status IN ('uploaded', 'attached', 'lost')
  AND ($2::uuid IS NULL OR root.origin_conversation = $2::uuid)
  AND ($3::timestamptz IS NULL OR (root.created_at, root.id) < ($3::timestamptz, $4::uuid))
ORDER BY root.created_at DESC, root.id DESC
LIMIT $5
