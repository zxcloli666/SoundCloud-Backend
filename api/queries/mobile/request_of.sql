SELECT state,
       status,
       error,
       expires_at <= now() AS "expired!"
FROM mobile_link_requests
WHERE id = $1
  AND session_id = $2
