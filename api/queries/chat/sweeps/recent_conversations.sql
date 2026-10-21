SELECT id, kind, last_seq
FROM chat.conversations
WHERE last_at > now() - interval '10 minutes'
  AND deleted_at IS NULL
  AND last_seq > 0
