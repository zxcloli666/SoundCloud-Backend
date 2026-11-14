SELECT track.sc_track_id
FROM track_storage_failures AS failure
JOIN tracks AS track ON track.id = failure.track_id
WHERE failure.failed_at < now() - INTERVAL '24 hours'
  AND track.storage_state = 'failed'
  AND track.needs_duration_resolve = false
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'indexing.track'
        AND job.dedup_key = track.sc_track_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_job_failures AS failure_log
      WHERE failure_log.kind = 'indexing.track'
        AND failure_log.failed_at > now() - $2::bigint * interval '1 second'
        AND failure_log.dedup_key = track.sc_track_id
  )
ORDER BY failure.failed_at
LIMIT $1
