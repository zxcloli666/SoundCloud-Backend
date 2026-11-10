SELECT root.id,
       root.kind,
       root.status,
       root.store,
       root.object_key,
       root.bytes,
       coalesce(root.delete_after > now(), false) AS "live!",
       thumb.id AS "thumb_id?",
       thumb.object_key AS "thumb_key?",
       thumb.bytes AS "thumb_bytes?"
FROM chat.media AS root
LEFT JOIN chat.media AS thumb
    ON thumb.parent_id = root.id
   AND thumb.kind = 'thumb'
WHERE root.id = $1
  AND root.owner_user = $2
  AND root.kind <> 'thumb'
