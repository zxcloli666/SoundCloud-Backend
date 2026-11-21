SELECT root.id,
       root.kind,
       root.status,
       root.store,
       root.object_key,
       root.bytes,
       coalesce(root.delete_after > now(), false) AS "live!",
       thumb.id AS "thumb_id?",
       thumb.object_key AS "thumb_key?",
       thumb.bytes AS "thumb_bytes?",
       motion.id AS "motion_id?",
       motion.object_key AS "motion_key?",
       motion.bytes AS "motion_bytes?"
FROM chat.media AS root
LEFT JOIN chat.media AS thumb
    ON thumb.parent_id = root.id
   AND thumb.kind = 'thumb'
LEFT JOIN chat.media AS motion
    ON motion.parent_id = root.id
   AND motion.kind = 'motion'
WHERE root.id = $1
  AND root.owner_user = $2
  AND root.kind NOT IN ('thumb', 'motion')
