SELECT coalesce(
    max(failed_at)
        + least(greatest(max(failed_at) - min(failed_at), interval '1 day'), interval '21 days')
        > now(),
    false
) AS "absent!"
FROM background_job_failures
WHERE kind = 'catalog.refresh'
  AND dedup_key = $1
  AND split_part(last_error, ':', 1) = 'not_found'
