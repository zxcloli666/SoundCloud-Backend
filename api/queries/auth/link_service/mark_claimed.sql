UPDATE link_requests
SET source_session_id = $2,
    target_session_id = $3,
    chat_bind         = $4,
    status            = 'claimed'
WHERE id = $1
