SELECT EXISTS (
    SELECT 1
    FROM chat.media_refs
    WHERE conversation_id = $1
) AS "left!"
