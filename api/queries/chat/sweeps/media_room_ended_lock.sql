SELECT media.id
FROM chat.media AS media
JOIN chat.conversations AS conversation
  ON conversation.id = media.origin_conversation
WHERE media.origin_conversation = $1
  AND media.kind <> 'thumb'
  AND media.status IN ('reserved', 'uploaded')
  AND conversation.kind = 'room'
  AND conversation.deleted_at IS NOT NULL
ORDER BY media.id
FOR UPDATE OF media
