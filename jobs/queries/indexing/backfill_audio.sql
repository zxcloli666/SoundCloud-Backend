SELECT track.sc_track_id AS "sc_track_id!",
       storage.uploaded_generation AS "uploaded_generation?"
FROM tracks AS track
LEFT JOIN storage_event_state AS storage
  ON storage.sc_track_id = track.sc_track_id
WHERE track.index_state = 'pending'
  AND track.storage_state = 'ok'
  AND track.s3_verified_at IS NOT NULL
  AND NOT track.needs_duration_resolve
  AND NOT track.pipeline_held
  AND track.created_at < now() - INTERVAL '5 minutes'
  AND NOT EXISTS (
      SELECT 1
      FROM audio_index_wire_state AS wire
      WHERE wire.sc_track_id = track.sc_track_id
        AND (
            (
                wire.upload_generation = storage.uploaded_generation
                AND wire.status IN ('terminal', 'reopenable')
            )
            OR (
                wire.status = 'pending'
                AND wire.dispatched_at > now() - $2::bigint * interval '1 second'
            )
        )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind IN ('indexing.track', 'indexing.dispatch_audio')
        AND job.dedup_key = track.sc_track_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_job_failures AS failure
      WHERE failure.kind IN ('indexing.track', 'indexing.dispatch_audio')
        AND failure.failed_at > now() - $3::bigint * interval '1 second'
        AND failure.dedup_key = track.sc_track_id
  )
ORDER BY track.index_priority, track.created_at, track.sc_track_id
LIMIT $1
