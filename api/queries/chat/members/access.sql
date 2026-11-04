SELECT c.kind,
       c.dm_key,
       c.key_epoch,
       c.last_seq,
       c.ttl_s,
       c.settings,
       m.role,
       m.state,
       m.accepted,
       m.joined_seq,
       m.hidden_before_seq,
       ARRAY(SELECT r.user_id
             FROM chat.members r
             WHERE r.conversation_id = c.id
               AND r.state IN ('active', 'invited')
             ORDER BY r.user_id) AS "recipients!",
       (SELECT p.state
        FROM chat.members p
        WHERE p.conversation_id = c.id
          AND p.user_id <> m.user_id
          AND c.kind = 'direct'
        LIMIT 1) AS peer_state
FROM chat.conversations c
JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2
WHERE c.id = $1
  AND c.deleted_at IS NULL
