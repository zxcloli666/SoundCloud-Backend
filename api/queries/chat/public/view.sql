SELECT c.id,
       c.created_by,
       c.created_at,
       c.last_seq,
       c.last_at,
       c.key_epoch,
       c.meta_ct,
       c.meta_epoch,
       c.settings,
       m.role AS "role?",
       m.state AS "state?",
       m.last_read_seq AS "last_read_seq?",
       m.muted_until AS "muted_until?",
       m.notify AS "notify?",
       m.hidden_before_seq AS "hidden_before_seq?",
       m.last_ping_seq AS "last_ping_seq?",
       m.joined_at AS "joined_at?",
       p.until AS "mute_until?",
       p.reason AS "mute_reason?"
FROM chat.conversations c
LEFT JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2
LEFT JOIN chat.public_mutes p ON p.user_id = $2 AND p.until > now()
WHERE c.id = $1
  AND c.kind = 'public'
  AND c.deleted_at IS NULL
