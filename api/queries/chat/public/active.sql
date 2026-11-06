SELECT EXISTS (
    SELECT 1
    FROM chat.members
    WHERE conversation_id = $1
      AND user_id = $2
      AND state = 'active'
) AS "active!"
