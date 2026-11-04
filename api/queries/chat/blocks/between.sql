SELECT EXISTS (
    SELECT 1
    FROM chat.blocks
    WHERE (user_id = $1 AND blocked_user = $2)
       OR (user_id = $2 AND blocked_user = $1)
) OR EXISTS (
    SELECT 1
    FROM user_blocked_artists
    WHERE kind = 'user'
      AND ((sc_user_id = $1 AND $2 = ANY (sc_user_ids)) OR (sc_user_id = $2 AND $1 = ANY (sc_user_ids)))
) AS "blocked!"
