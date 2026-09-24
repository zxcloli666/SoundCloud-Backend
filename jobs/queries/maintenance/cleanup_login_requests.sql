WITH expired AS (
    SELECT id
    FROM login_requests
    WHERE expires_at < now()
      AND (
          status <> 'processing'
          OR expires_at < now() - interval '1 hour'
      )
    ORDER BY expires_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
DELETE FROM login_requests AS request
USING expired
WHERE request.id = expired.id
