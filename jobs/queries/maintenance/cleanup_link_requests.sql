WITH expired AS (
    SELECT id
    FROM link_requests
    WHERE expires_at < now()
    ORDER BY expires_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
DELETE FROM link_requests AS request
USING expired
WHERE request.id = expired.id
