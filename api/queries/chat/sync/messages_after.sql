SELECT
    picked.conversation_id AS "conversation_id!",
    picked.seq AS "seq!",
    picked.client_id AS "client_id!",
    picked.sender_user AS "sender_user!",
    picked.sender_device AS "sender_device?",
    picked.class AS "class!",
    picked.epoch AS "epoch!",
    picked.target_seq AS "target_seq?",
    picked.body AS "body?",
    picked.created_at AS "created_at!",
    picked.expires_at AS "expires_at?"
FROM (
    SELECT
        w.ord,
        w.conversation_id,
        m.seq,
        m.client_id,
        m.sender_user,
        m.sender_device,
        m.class,
        m.epoch,
        m.target_seq,
        m.body,
        m.created_at,
        m.expires_at,
        sum(COALESCE(octet_length(m.body), 0) + 256) OVER (ORDER BY w.ord, m.seq) AS running
    FROM unnest($1::uuid[], $2::bigint[]) WITH ORDINALITY AS w(conversation_id, after_seq, ord)
    CROSS JOIN LATERAL (
        SELECT
            page.seq,
            page.client_id,
            page.sender_user,
            page.sender_device,
            page.class,
            page.epoch,
            page.target_seq,
            page.body,
            page.created_at,
            page.expires_at
        FROM chat.messages page
        WHERE page.conversation_id = w.conversation_id
          AND page.seq > w.after_seq
          AND (page.expires_at IS NULL OR page.expires_at > now())
        ORDER BY page.seq
        LIMIT $3
    ) m
) picked
WHERE picked.running <= $4
ORDER BY picked.ord, picked.seq
