UPDATE lyrics_repair AS repair
SET status = $5::varchar(16),
    outcome = NULLIF($6::text, ''),
    next_run_at = CASE
        WHEN $5::varchar(16) = 'pending' THEN now() + $7::bigint * interval '1 second'
        ELSE repair.next_run_at
    END,
    claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_expires_at = NULL,
    settled_at = CASE WHEN $5::varchar(16) = 'done' THEN now() END,
    updated_at = now()
FROM background_jobs AS job
WHERE job.id = $1
  AND job.generation = $2
  AND job.lease_generation = $2
  AND job.lease_id = $3
  AND job.lease_expires_at > now()
  AND repair.sc_track_id = $4
  AND repair.kind = 'refetch'
  AND repair.status = 'pending'
  AND repair.claim_job_id = $1
  AND repair.claim_job_generation = $2
  AND repair.claim_job_lease_id = $3
  AND repair.claim_expires_at > now()
RETURNING repair.sc_track_id
