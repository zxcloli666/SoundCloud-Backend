SELECT
    c.id,
    c.kind,
    c.last_seq,
    CASE WHEN c.kind = 'direct' THEN (
        SELECT p.user_id
        FROM chat.members p
        WHERE p.conversation_id = c.id
          AND p.user_id <> $2
          AND p.state IN ('active', 'invited')
        LIMIT 1
    ) END AS "peer?"
FROM chat.conversations c
JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2 AND m.state IN ('active', 'invited')
WHERE c.id = ANY($1)
  AND c.deleted_at IS NULL
