WITH old AS (
    SELECT user_id, last_read_seq
    FROM chat.members
    WHERE conversation_id = $1
      AND user_id = $2
      AND state = 'active'
      AND last_read_seq < $3
    FOR UPDATE
)
UPDATE chat.members AS m
SET last_read_seq = $3
FROM old
WHERE m.conversation_id = $1
  AND m.user_id = old.user_id
RETURNING (m.last_ping_seq > old.last_read_seq AND m.last_ping_seq <= $3) AS "cleared!"
