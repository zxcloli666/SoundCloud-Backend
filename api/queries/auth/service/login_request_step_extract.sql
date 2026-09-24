UPDATE login_requests
SET step = 'extract'
WHERE id = $1
  AND status = 'processing'
