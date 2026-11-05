SELECT id,
       code_verifier,
       oauth_app_id,
       target_session_id,
       status,
       username,
       error,
       retry_count,
       expires_at
FROM login_requests
WHERE state = $1
