SELECT EXISTS(
    SELECT 1
    FROM background_job_failures
    WHERE kind = 'catalog.refresh'
      AND dedup_key = $1
      AND attempts < max_attempts
      AND failed_at > now() - interval '30 minutes'
) AS "unavailable!"
