DELETE FROM chat.media_refs AS ref
USING chat.media AS media
WHERE ref.conversation_id = $1
  AND ref.seq = 0
  AND media.id = ref.media_id
  AND media.kind = 'avatar'
  AND ref.media_id <> ALL($2::uuid[])
RETURNING ref.media_id
