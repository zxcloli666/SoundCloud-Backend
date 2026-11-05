WITH dropped AS (
    DELETE FROM chat.media_refs
    WHERE media_id = $1
      AND conversation_id = $2
      AND seq = 0
    RETURNING media_id
)
UPDATE chat.media
SET status = 'uploaded',
    delete_after = now() + interval '1 hour'
WHERE (id = $1 OR parent_id = $1)
  AND status = 'attached'
  AND EXISTS (SELECT 1 FROM dropped)
