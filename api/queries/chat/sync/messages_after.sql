SELECT
    w.conversation_id AS "conversation_id!",
    m.seq AS "seq!",
    m.client_id AS "client_id!",
    m.sender_user AS "sender_user!",
    m.sender_device AS "sender_device?",
    m.class AS "class!",
    m.epoch AS "epoch!",
    m.target_seq AS "target_seq?",
    m.body AS "body?",
    m.created_at AS "created_at!",
    m.expires_at AS "expires_at?"
FROM unnest($1::uuid[], $2::bigint[]) AS w(conversation_id, after_seq)
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
        page.expires_at,
        sum(COALESCE(octet_length(page.body), 0) + 256) OVER (ORDER BY page.seq) AS running
    FROM chat.messages page
    WHERE page.conversation_id = w.conversation_id
      AND page.seq > w.after_seq
      AND (page.expires_at IS NULL OR page.expires_at > now())
    ORDER BY page.seq
    LIMIT $3
) m
WHERE m.running <= $4
ORDER BY w.conversation_id, m.seq
