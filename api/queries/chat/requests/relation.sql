SELECT EXISTS (
    SELECT 1
    FROM chat.members a
    JOIN chat.members b ON b.conversation_id = a.conversation_id AND b.user_id = $2
    JOIN chat.conversations c ON c.id = a.conversation_id
    WHERE a.user_id = $1
      AND a.state = 'active'
      AND b.state = 'active'
      AND a.accepted
      AND b.accepted
      AND c.kind IN ('direct', 'group')
      AND c.deleted_at IS NULL
) AS "contact!",
EXISTS (
    SELECT 1
    FROM user_followings
    WHERE user_id = ANY ($3)
      AND target_user_urn = $4
      AND wanted_state
) AS "follows!"
