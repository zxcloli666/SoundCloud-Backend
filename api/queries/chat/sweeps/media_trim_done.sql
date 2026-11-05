DELETE FROM chat.media_trim
WHERE conversation_id = $1
  AND queued_at = $2
