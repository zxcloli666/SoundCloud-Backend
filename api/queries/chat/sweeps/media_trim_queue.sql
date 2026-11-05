SELECT conversation_id, queued_at
FROM chat.media_trim
ORDER BY queued_at
LIMIT $1
