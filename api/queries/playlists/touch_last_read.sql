WITH stamp AS (
    SELECT EXISTS (
        SELECT 1
        FROM playlists
        WHERE urn = $1::text
          AND (last_read_at IS NULL OR last_read_at < now() - INTERVAL '6 hours')
    ) AS due
)
INSERT INTO entity_reads (entity_urn, last_read_at)
SELECT $1::text, now()
FROM stamp
WHERE NOT EXISTS (
    SELECT 1
    FROM entity_reads
    WHERE entity_urn = $1::text
      AND last_read_at >= now() - CASE WHEN stamp.due THEN INTERVAL '1 minute' ELSE INTERVAL '1 hour' END
)
ON CONFLICT (entity_urn) DO UPDATE
SET last_read_at = EXCLUDED.last_read_at
WHERE entity_reads.last_read_at < EXCLUDED.last_read_at - INTERVAL '1 minute'
