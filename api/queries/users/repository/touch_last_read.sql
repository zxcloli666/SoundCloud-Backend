INSERT INTO entity_reads (entity_urn, last_read_at)
SELECT $1::text, now()
WHERE NOT EXISTS (
    SELECT 1
    FROM entity_reads
    WHERE entity_urn = $1::text
      AND last_read_at >= now() - INTERVAL '1 hour'
)
ON CONFLICT (entity_urn) DO UPDATE
SET last_read_at = EXCLUDED.last_read_at
WHERE entity_reads.last_read_at < EXCLUDED.last_read_at - INTERVAL '1 hour'
