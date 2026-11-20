SELECT requested_at + make_interval(secs => $2::double precision) AS "available_at!"
FROM library_sync_requests
WHERE sc_user_id = $1
