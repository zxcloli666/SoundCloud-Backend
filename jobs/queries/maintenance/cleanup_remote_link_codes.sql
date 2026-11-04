WITH expired AS (
    SELECT id
    FROM remote_link_codes
    WHERE expires_at < now()
    ORDER BY expires_at
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
DELETE FROM remote_link_codes AS code
USING expired
WHERE code.id = expired.id
