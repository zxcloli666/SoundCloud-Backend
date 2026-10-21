SELECT
    c.id,
    c.kind,
    c.last_seq,
    c.key_epoch,
    c.deleted_at IS NOT NULL AS "deleted!",
    m.state AS "state?",
    m.joined_seq AS "joined_seq?",
    m.hidden_before_seq AS "hidden_before_seq?"
FROM chat.conversations c
LEFT JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2
WHERE c.id = ANY($1)
