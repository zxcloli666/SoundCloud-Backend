UPDATE login_requests
SET status = 'failed',
    step   = NULL,
    error  = $2
WHERE id = $1
  AND status = 'processing'
