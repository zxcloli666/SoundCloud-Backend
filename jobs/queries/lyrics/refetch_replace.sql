WITH owned AS MATERIALIZED (
    SELECT repair.sc_track_id
    FROM lyrics_repair AS repair
    JOIN background_jobs AS job
      ON job.id = $1
     AND job.generation = $2
     AND job.lease_generation = $2
     AND job.lease_id = $3
     AND job.lease_expires_at > now()
    WHERE repair.sc_track_id = $4
      AND repair.kind = 'refetch'
      AND repair.status = 'pending'
      AND repair.claim_job_id = $1
      AND repair.claim_job_generation = $2
      AND repair.claim_job_lease_id = $3
      AND repair.claim_expires_at > now()
    FOR UPDATE OF repair
), previous AS MATERIALIZED (
    SELECT cache.plain_text
    FROM lyrics_cache AS cache
    JOIN owned ON owned.sc_track_id = cache.sc_track_id
    WHERE cache.content_generation = $5
), replaced AS (
    UPDATE lyrics_cache AS cache
    SET plain_text = NULLIF(btrim($6::text, E' \t\n\r\f\x0B'), ''),
        embedded_at = NULL,
        embedding_state = NULL,
        content_generation = cache.content_generation + 1,
        updated_at = now()
    FROM owned
    WHERE cache.sc_track_id = owned.sc_track_id
      AND cache.content_generation = $5
      AND NULLIF(btrim($6::text, E' \t\n\r\f\x0B'), '') IS NOT NULL
    RETURNING cache.sc_track_id
)
UPDATE lyrics_repair AS repair
SET status = 'done',
    outcome = CASE WHEN EXISTS (SELECT 1 FROM replaced) THEN 'replaced' ELSE 'stale' END,
    old_plain_text = (SELECT previous.plain_text FROM previous),
    claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_expires_at = NULL,
    settled_at = now(),
    updated_at = now()
FROM owned
WHERE repair.sc_track_id = owned.sc_track_id
RETURNING repair.outcome AS "outcome!"
