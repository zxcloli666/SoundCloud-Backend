SELECT
    c.kind,
    COALESCE(m.state = 'active', false) AS "member!",
    COALESCE(u.typing, true) AS "typing!",
    c.kind = 'direct' AND EXISTS (
        SELECT 1
        FROM chat.members peer
        WHERE peer.conversation_id = c.id
          AND peer.user_id <> $2
          AND (
              EXISTS (
                  SELECT 1
                  FROM chat.blocks b
                  WHERE (b.user_id = peer.user_id AND b.blocked_user = $2)
                     OR (b.user_id = $2 AND b.blocked_user = peer.user_id)
              )
              OR EXISTS (
                  SELECT 1
                  FROM user_blocked_artists x
                  WHERE x.kind = 'user'
                    AND ((x.sc_user_id = peer.user_id AND $2 = ANY (x.sc_user_ids))
                      OR (x.sc_user_id = $2 AND peer.user_id = ANY (x.sc_user_ids)))
              )
          )
    ) AS "blocked!"
FROM chat.conversations c
LEFT JOIN chat.members m ON m.conversation_id = c.id AND m.user_id = $2
LEFT JOIN chat.users u ON u.user_id = $2
WHERE c.id = $1
  AND c.deleted_at IS NULL
