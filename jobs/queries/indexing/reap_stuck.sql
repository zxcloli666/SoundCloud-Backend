SELECT track.sc_track_id
FROM tracks AS track
WHERE track.created_at < now() - INTERVAL '5 minutes'
  AND track.needs_duration_resolve = false
  AND (
    track.storage_state = 'pending'
    OR (
      $4::boolean
      AND track.index_state = 'pending'
      AND track.storage_state = 'ok'
      AND track.s3_verified_at IS NOT NULL
      AND NOT EXISTS (
          SELECT 1
          FROM audio_index_wire_state AS settled
          JOIN storage_event_state AS storage
            ON storage.sc_track_id = settled.sc_track_id
           AND storage.uploaded_generation = settled.upload_generation
          WHERE settled.sc_track_id = track.sc_track_id
            AND settled.status IN ('terminal', 'reopenable')
      )
    )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM audio_index_wire_state AS wire
      WHERE wire.sc_track_id = track.sc_track_id
        AND wire.status = 'pending'
        AND wire.dispatched_at > now() - $2::bigint * interval '1 second'
  )
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
        AND failure.failed_at > now() - $3::bigint * interval '1 second'
        AND failure.dedup_key = track.sc_track_id
  )
ORDER BY track.index_priority, track.created_at
LIMIT $1
