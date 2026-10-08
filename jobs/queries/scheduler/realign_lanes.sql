UPDATE background_jobs AS job
SET lane = expected.lane,
    updated_at = now()
FROM unnest($1::text[], $2::text[]) AS expected(kind, lane)
WHERE job.kind = expected.kind
  AND job.lane <> expected.lane
  AND job.lease_id IS NULL
