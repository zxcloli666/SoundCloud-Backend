SELECT EXISTS (
    SELECT 1
    FROM chat.media_refs
    WHERE conversation_id = $1
      AND seq > 0
      AND seq <= $2
) AS "left!"
