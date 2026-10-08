SELECT count(*) AS "queued!"
FROM (
    SELECT 1
    FROM background_jobs
    WHERE kind = $1
      AND dedup_key IS NOT NULL
    LIMIT $2
) AS queued
