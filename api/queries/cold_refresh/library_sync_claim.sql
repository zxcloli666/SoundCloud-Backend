INSERT INTO library_sync_requests AS request (sc_user_id, requested_at)
VALUES ($1, now())
ON CONFLICT (sc_user_id) DO UPDATE
SET requested_at = EXCLUDED.requested_at
WHERE request.requested_at <= now() - make_interval(secs => $2::double precision)
RETURNING request.requested_at + make_interval(secs => $2::double precision) AS "available_at!"
