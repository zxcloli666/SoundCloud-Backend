DELETE FROM chat.media_refs AS ref
USING chat.media AS media
WHERE ref.conversation_id = $1
  AND ref.seq = 0
  AND media.id = ref.media_id
  AND media.kind = 'backdrop'
  AND ref.media_id <> ALL ($2::uuid[])
  AND (media.owner_user = $3 OR ref.media_id = ANY ($4::uuid[]))
RETURNING ref.media_id
