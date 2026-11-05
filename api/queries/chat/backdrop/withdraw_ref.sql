DELETE FROM chat.media_refs AS ref
USING chat.media AS media
WHERE ref.media_id = $1
  AND ref.conversation_id = $2
  AND ref.seq = 0
  AND media.id = ref.media_id
  AND media.kind = 'backdrop'
RETURNING ref.media_id
