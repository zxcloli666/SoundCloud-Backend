INSERT INTO link_requests
(id, claim_token, mode, source_session_id, target_session_id, status, expires_at, chat_bind)
VALUES ($1, $2, $3, $4, NULL, 'pending',
        $5, $6) RETURNING id, mode, source_session_id, target_session_id, status, error, expires_at
