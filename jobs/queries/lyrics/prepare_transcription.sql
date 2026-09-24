WITH track AS MATERIALIZED (
    SELECT current.sc_track_id,
           current.transcribe_state,
           current.storage_state,
           current.needs_duration_resolve
    FROM tracks AS current
    WHERE current.sc_track_id = $1
    FOR UPDATE
), storage AS MATERIALIZED (
    SELECT event.sc_track_id,
           event.uploaded_generation
    FROM storage_event_state AS event
    JOIN track ON track.sc_track_id = event.sc_track_id
    WHERE event.uploaded_generation = $2
      AND track.storage_state = 'ok'
      AND NOT track.needs_duration_resolve
    FOR UPDATE OF event
), lyrics AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.plain_text,
           cache.language
    FROM lyrics_cache AS cache
    JOIN track ON track.sc_track_id = cache.sc_track_id
    WHERE NULLIF(btrim(cache.plain_text), '') IS NOT NULL
      AND NULLIF(btrim(cache.synced_lrc), '') IS NULL
), superseded AS MATERIALIZED (
    SELECT state.sc_track_id
    FROM transcription_wire_state AS state
    JOIN storage ON storage.sc_track_id = state.sc_track_id
    WHERE state.upload_generation < storage.uploaded_generation
      AND (
          state.status = 'reopenable'
          OR (
              state.status = 'quarantined'
              AND state.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
          )
      )
    FOR UPDATE OF state
), claimed AS (
    INSERT INTO transcription_wire_state (
        sc_track_id,
        status,
        upload_generation,
        attempt,
        dispatched_at
    )
    SELECT storage.sc_track_id, 'pending', storage.uploaded_generation, 1, now()
    FROM storage
    JOIN track ON track.sc_track_id = storage.sc_track_id
    JOIN lyrics ON lyrics.sc_track_id = storage.sc_track_id
    WHERE track.transcribe_state IS NULL
       OR track.transcribe_state = 'pending'
       OR (
           track.transcribe_state = 'quarantined'
           AND EXISTS (SELECT 1 FROM superseded)
       )
    ON CONFLICT (sc_track_id) DO UPDATE
    SET status = 'pending',
        upload_generation = EXCLUDED.upload_generation,
        attempt = CASE
            WHEN transcription_wire_state.upload_generation = EXCLUDED.upload_generation
                THEN transcription_wire_state.attempt
            ELSE 1
        END,
        reopen_count = CASE
            WHEN transcription_wire_state.upload_generation = EXCLUDED.upload_generation
                THEN transcription_wire_state.reopen_count
            ELSE 0
        END,
        dispatched_at = CASE
            WHEN transcription_wire_state.status = 'pending'
                THEN transcription_wire_state.dispatched_at
            ELSE now()
        END,
        completed_at = NULL,
        reason = NULL,
        quarantine_reason = NULL,
        result_rank = NULL,
        sync_version = CASE
            WHEN transcription_wire_state.upload_generation = EXCLUDED.upload_generation
                THEN transcription_wire_state.sync_version
        END,
        reopened_for_sync_version = CASE
            WHEN transcription_wire_state.upload_generation = EXCLUDED.upload_generation
                THEN transcription_wire_state.reopened_for_sync_version
        END,
        result_stream_sequence = NULL,
        result_published_at = NULL,
        updated_at = now()
    WHERE (
            transcription_wire_state.upload_generation = EXCLUDED.upload_generation
            AND (
                transcription_wire_state.status = 'pending'
                OR (
                    transcription_wire_state.status = 'reopenable'
                    AND transcription_wire_state.reason = 'dispatch_publish_failed'
                )
            )
        )
       OR EXISTS (SELECT 1 FROM superseded)
    RETURNING sc_track_id,
              upload_generation,
              attempt
), prepared AS (
    UPDATE tracks AS current
    SET transcribe_state = 'pending',
        transcribe_at = CASE
            WHEN current.transcribe_state = 'pending' THEN current.transcribe_at
            ELSE now()
        END,
        updated_at = now()
    FROM claimed
    WHERE current.sc_track_id = claimed.sc_track_id
    RETURNING claimed.sc_track_id,
              claimed.upload_generation,
              claimed.attempt
), marked AS (
    UPDATE storage_event_state AS event
    SET transcription_generation = prepared.upload_generation,
        updated_at = now()
    FROM prepared
    WHERE event.sc_track_id = prepared.sc_track_id
      AND event.uploaded_generation = prepared.upload_generation
    RETURNING prepared.sc_track_id,
              prepared.attempt
)
SELECT marked.attempt AS "attempt!",
       lyrics.plain_text AS "plain_text!",
       lyrics.language
FROM marked
JOIN lyrics ON lyrics.sc_track_id = marked.sc_track_id
