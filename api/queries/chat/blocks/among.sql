SELECT other AS "user_id!"
FROM unnest($2::text[]) AS other
WHERE EXISTS (
    SELECT 1
    FROM chat.blocks
    WHERE (user_id = $1 AND blocked_user = other)
       OR (user_id = other AND blocked_user = $1)
) OR EXISTS (
    SELECT 1
    FROM user_blocked_artists
    WHERE kind = 'user'
      AND ((sc_user_id = $1 AND other = ANY (sc_user_ids)) OR (sc_user_id = other AND $1 = ANY (sc_user_ids)))
)
