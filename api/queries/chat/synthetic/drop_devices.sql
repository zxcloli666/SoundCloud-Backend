DELETE FROM chat.devices
WHERE id IN (
    SELECT candidate.id
    FROM chat.devices AS candidate
    WHERE candidate.user_id LIKE 'lt:%'
    LIMIT $1
)
RETURNING id, user_id, session_id
