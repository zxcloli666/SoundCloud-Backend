UPDATE mobile_link_requests
SET status = $2,
    error = $3
WHERE id = $1
  AND status = 'processing'
