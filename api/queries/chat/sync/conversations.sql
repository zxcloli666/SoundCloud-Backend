SELECT
    c.id,
    c.kind,
    c.last_seq,
    c.key_epoch,
    c.deleted_at IS NOT NULL AS "deleted!",
    m.state AS "state?",
    m.joined_seq AS "joined_seq?",
    m.hidden_before_seq AS "hidden_before_seq?",
    hashtextextended(
        concat_ws(
            '|',
            c.settings::text,
            coalesce(c.ttl_s::text, ''),
            coalesce(m.role, ''),
            coalesce(m.state, ''),
            coalesce(m.accepted::text, ''),
            coalesce(extract(epoch FROM m.muted_until)::text, ''),
            coalesce(m.notify, ''),
            coalesce(extract(epoch FROM m.pinned_at)::text, ''),
            coalesce(m.archived::text, ''),
            coalesce(m.hidden_before_seq::text, ''),
            coalesce((m.last_ping_seq > m.last_read_seq)::text, '')
        ),
        0
    ) AS "rev!"
FROM chat.conversations c
LEFT JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2
WHERE c.id = ANY($1)
