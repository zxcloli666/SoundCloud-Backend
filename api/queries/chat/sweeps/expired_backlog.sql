SELECT min(expires_at) AS oldest
FROM chat.messages
WHERE expires_at IS NOT NULL
  AND expires_at <= now()
