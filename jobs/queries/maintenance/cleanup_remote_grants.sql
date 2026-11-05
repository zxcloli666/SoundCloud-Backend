WITH expired AS (
    SELECT id
    FROM remote_grants
    WHERE external_id IS NULL
      AND (
          refresh_expires_at < now()
          OR (access_hash IS NULL AND code_expires_at < now())
      )
    FOR UPDATE SKIP LOCKED
    LIMIT $1
)
DELETE FROM remote_grants AS stale
USING expired
WHERE stale.id = expired.id
