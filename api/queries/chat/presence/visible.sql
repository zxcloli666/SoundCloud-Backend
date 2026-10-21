SELECT EXISTS (
    SELECT 1
    FROM chat.users shown
    WHERE shown.user_id = $2
      AND shown.presence_share
      AND (
          EXISTS (
              SELECT 1
              FROM chat.members mine
              JOIN chat.conversations c
                  ON c.id = mine.conversation_id
                  AND c.kind = 'direct'
                  AND c.deleted_at IS NULL
              JOIN chat.members peer
                  ON peer.conversation_id = mine.conversation_id
                  AND peer.user_id = $2
                  AND peer.state = 'active'
                  AND peer.accepted
              WHERE mine.user_id = $1
                AND mine.state = 'active'
                AND mine.accepted
          )
          OR (
              EXISTS (
                  SELECT 1
                  FROM user_followings f
                  WHERE f.user_id IN ($1, 'soundcloud:users:' || $1)
                    AND f.target_user_urn = 'soundcloud:users:' || $2
                    AND f.wanted_state
              )
              AND EXISTS (
                  SELECT 1
                  FROM user_followings f
                  WHERE f.user_id IN ($2, 'soundcloud:users:' || $2)
                    AND f.target_user_urn = 'soundcloud:users:' || $1
                    AND f.wanted_state
              )
          )
      )
      AND NOT EXISTS (
          SELECT 1
          FROM chat.blocks b
          WHERE (b.user_id = $1 AND b.blocked_user = $2)
             OR (b.user_id = $2 AND b.blocked_user = $1)
      )
      AND NOT EXISTS (
          SELECT 1
          FROM user_blocked_artists x
          WHERE x.kind = 'user'
            AND ((x.sc_user_id = $1 AND $2 = ANY (x.sc_user_ids))
              OR (x.sc_user_id = $2 AND $1 = ANY (x.sc_user_ids)))
      )
) AS "visible!"
