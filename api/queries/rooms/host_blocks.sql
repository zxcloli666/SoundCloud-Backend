SELECT EXISTS (
    SELECT 1
    FROM user_blocked_artists
    WHERE sc_user_id = ANY ($1)
      AND $2 = ANY (sc_user_ids)
) OR EXISTS (
    SELECT 1
    FROM chat.blocks
    WHERE user_id = $3
      AND blocked_user = $2
) AS "blocked!"
