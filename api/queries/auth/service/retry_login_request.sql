UPDATE login_requests
SET status = 'pending',
    step = NULL,
    error = NULL,
    state = $2,
    code_verifier = $3,
    oauth_app_id = $4,
    redirect_url = $5,
    retry_count = retry_count + 1,
    expires_at = $6
WHERE id = $1
  AND status = 'processing'
RETURNING id
