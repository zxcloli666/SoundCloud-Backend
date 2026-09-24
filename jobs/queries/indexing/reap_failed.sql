SELECT track.sc_track_id
FROM tracks AS track
WHERE track.storage_state = 'failed'
  AND track.needs_duration_resolve = false
  AND track.updated_at < now() - INTERVAL '24 hours'
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'indexing.track'
        AND job.dedup_key = track.sc_track_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_job_failures AS failure
      WHERE failure.kind = 'indexing.track'
        AND failure.failed_at > now() - $2::bigint * interval '1 second'
        AND failure.dedup_key = track.sc_track_id
  )
ORDER BY track.updated_at
LIMIT $1
