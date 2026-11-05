DELETE FROM chat.media_refs AS ref
USING chat.media AS media
WHERE ref.media_id = ANY($1::uuid[])
  AND media.id = ref.media_id
  AND media.status = 'deleting'
  AND (media.restorable_until IS NULL OR media.restorable_until <= now())
RETURNING ref.conversation_id, ref.media_id, ref.seq = 0 AND media.kind = 'backdrop' AS "backdrop!"
