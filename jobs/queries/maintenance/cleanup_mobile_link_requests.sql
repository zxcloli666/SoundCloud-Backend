WITH expired AS (
    SELECT id
    FROM mobile_link_requests
    WHERE expires_at < now()
      AND (
          status <> 'processing'
          OR expires_at < now() - interval '1 hour'
      )
    ORDER BY expires_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
DELETE FROM mobile_link_requests AS request
USING expired
WHERE request.id = expired.id
