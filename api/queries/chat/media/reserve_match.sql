SELECT root.id,
       root.kind,
       root.store,
       root.object_key,
       root.bytes,
       root.origin_conversation,
       thumb.id AS "thumb_id?",
       thumb.object_key AS "thumb_key?",
       thumb.bytes AS "thumb_bytes?"
FROM chat.media AS root
LEFT JOIN chat.media AS thumb
    ON thumb.parent_id = root.id
   AND thumb.kind = 'thumb'
WHERE root.owner_user = $1
  AND root.upload_id = $2
  AND root.status = 'reserved'
  AND root.kind <> 'thumb'
