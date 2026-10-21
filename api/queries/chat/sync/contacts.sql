SELECT DISTINCT contact.user_id AS "user_id!"
FROM (
    SELECT peer.user_id
    FROM chat.members mine
    JOIN chat.conversations c
        ON c.id = mine.conversation_id
        AND c.kind = 'direct'
        AND c.deleted_at IS NULL
    JOIN chat.members peer
        ON peer.conversation_id = mine.conversation_id
        AND peer.user_id <> mine.user_id
        AND peer.state = 'active'
        AND peer.accepted
    WHERE mine.user_id = $1
      AND mine.state = 'active'
      AND mine.accepted
    UNION
    SELECT split_part(outgoing.target_user_urn, ':', 3)
    FROM user_followings outgoing
    JOIN user_followings incoming
        ON incoming.user_id = split_part(outgoing.target_user_urn, ':', 3)
        AND incoming.target_user_urn = 'soundcloud:users:' || $1
        AND incoming.wanted_state
    WHERE outgoing.user_id = $1
      AND outgoing.wanted_state
) contact (user_id)
JOIN chat.users u ON u.user_id = contact.user_id AND u.presence_share
WHERE NOT EXISTS (
    SELECT 1
    FROM chat.blocks b
    WHERE (b.user_id = $1 AND b.blocked_user = contact.user_id)
       OR (b.user_id = contact.user_id AND b.blocked_user = $1)
)
  AND NOT EXISTS (
    SELECT 1
    FROM user_blocked_artists x
    WHERE x.kind = 'user'
      AND ((x.sc_user_id = $1 AND contact.user_id = ANY (x.sc_user_ids))
        OR (x.sc_user_id = contact.user_id AND $1 = ANY (x.sc_user_ids)))
)
