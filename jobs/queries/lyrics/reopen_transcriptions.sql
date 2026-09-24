WITH newest AS MATERIALIZED (
    SELECT seen.sync_version
    FROM transcription_sync_versions AS seen
    ORDER BY seen.first_seen_at DESC, seen.sync_version DESC
    LIMIT 1
), candidates AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM transcription_wire_state AS wire
    JOIN tracks AS track
      ON track.sc_track_id = wire.sc_track_id
    JOIN storage_event_state AS event
      ON event.sc_track_id = wire.sc_track_id
    JOIN lyrics_cache AS lyrics
      ON lyrics.sc_track_id = wire.sc_track_id
    WHERE wire.status IN ('rejected', 'reopenable')
      AND wire.completed_at < now() - $1::bigint * interval '1 second'
      AND event.uploaded_generation = wire.upload_generation
      AND track.storage_state = 'ok'
      AND NOT track.needs_duration_resolve
      AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
      AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
      AND (
          (wire.status = 'reopenable' AND wire.reopen_count < $2::integer)
          OR (
              wire.status = 'rejected'
              AND (
                  (
                      wire.sync_version IS DISTINCT FROM (SELECT sync_version FROM newest)
                      AND wire.reopened_for_sync_version
                          IS DISTINCT FROM (SELECT sync_version FROM newest)
                  )
                  OR wire.completed_at < now() - $3::bigint * interval '1 day'
              )
          )
      )
    ORDER BY wire.completed_at, wire.sc_track_id
    FOR UPDATE OF track, wire SKIP LOCKED
    LIMIT $4
), reopened AS (
    UPDATE transcription_wire_state AS wire
    SET status = 'pending',
        attempt = wire.attempt + 1,
        reopen_count = wire.reopen_count
            + CASE WHEN wire.status = 'reopenable' THEN 1 ELSE 0 END,
        reopened_for_sync_version = CASE
            WHEN wire.status = 'rejected' THEN (SELECT sync_version FROM newest)
            ELSE wire.reopened_for_sync_version
        END,
        dispatched_at = now(),
        completed_at = NULL,
        reason = NULL,
        quarantine_reason = NULL,
        result_rank = NULL,
        result_stream_sequence = NULL,
        result_published_at = NULL,
        updated_at = now()
    FROM candidates
    WHERE wire.sc_track_id = candidates.sc_track_id
    RETURNING wire.sc_track_id,
              wire.upload_generation
), tracked AS (
    UPDATE tracks AS track
    SET transcribe_state = 'pending',
        transcribe_at = now(),
        updated_at = now()
    FROM reopened
    WHERE track.sc_track_id = reopened.sc_track_id
    RETURNING track.sc_track_id
)
SELECT reopened.sc_track_id AS "sc_track_id!",
       reopened.upload_generation AS "uploaded_generation!"
FROM reopened
