UPDATE mobile_link_requests
SET status = 'processing'
WHERE id = $1
  AND session_id = $2
  AND state = $3
  AND status = 'pending'
  AND expires_at > now()
RETURNING id,
          code_verifier,
          oauth_app_id,
          expected_soundcloud_user_id,
          device_id
