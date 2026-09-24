WITH track AS MATERIALIZED (
    SELECT sc_track_id,
           storage_state,
           index_state,
           transcribe_state,
           duration_ms,
           needs_duration_resolve
    FROM tracks
    WHERE sc_track_id = $5
    FOR UPDATE
), locked_transcription AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.upload_generation
    FROM transcription_wire_state AS state
    JOIN track ON track.sc_track_id = state.sc_track_id
    FOR UPDATE OF state
), locked_index AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.upload_generation
    FROM audio_index_wire_state AS state
    JOIN track ON track.sc_track_id = state.sc_track_id
    FOR UPDATE OF state
), accepted AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM track
    ON CONFLICT DO NOTHING
    RETURNING 1
), advanced AS (
    INSERT INTO storage_event_state (
        sc_track_id,
        stream,
        stream_sequence,
        event_published_at,
        uploaded_generation
    )
    SELECT $5,
           $2,
           $3,
           $4,
           COALESCE((
               SELECT transcription.upload_generation
               FROM transcription_wire_state AS transcription
               WHERE transcription.sc_track_id = $5
           ), 0) + 1
    FROM accepted
    ON CONFLICT (sc_track_id) DO UPDATE
    SET stream = EXCLUDED.stream,
        stream_sequence = EXCLUDED.stream_sequence,
        event_published_at = EXCLUDED.event_published_at,
        uploaded_generation = GREATEST(
            storage_event_state.uploaded_generation + 1,
            EXCLUDED.uploaded_generation
        ),
        updated_at = now()
    WHERE (storage_event_state.event_published_at, storage_event_state.stream_sequence)
          < (EXCLUDED.event_published_at, EXCLUDED.stream_sequence)
    RETURNING uploaded_generation
), quarantined AS (
    UPDATE transcription_wire_state AS transcription
    SET status = 'quarantined',
        quarantine_reason = 'new_upload_during_pending',
        updated_at = now()
    FROM advanced
    JOIN locked_transcription
      ON locked_transcription.status = 'pending'
     AND locked_transcription.upload_generation <> advanced.uploaded_generation
    WHERE transcription.sc_track_id = locked_transcription.sc_track_id
    RETURNING transcription.sc_track_id
), index_quarantined AS (
    UPDATE audio_index_wire_state AS wire
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = 'new_upload_during_pending',
        updated_at = now()
    FROM advanced
    JOIN locked_index
      ON locked_index.status = 'pending'
     AND locked_index.upload_generation <> advanced.uploaded_generation
    WHERE wire.sc_track_id = locked_index.sc_track_id
    RETURNING wire.sc_track_id
), decision AS (
    SELECT advanced.uploaded_generation,
           track.storage_state = 'too_long'
               OR track.index_state = 'too_long'
               OR ($7 > 0 AND track.duration_ms > $7) AS too_long,
           NOT track.needs_duration_resolve AS dispatch_audio,
           track.index_state = 'indexed' AS reindex,
           track.transcribe_state IS NULL
               AND EXISTS (
                   SELECT 1
                   FROM lyrics_cache AS lyrics
                   WHERE lyrics.sc_track_id = track.sc_track_id
                     AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
                     AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM transcription_wire_state
                   WHERE sc_track_id = $5
               )
               AND NOT track.needs_duration_resolve AS dispatch_transcription
    FROM track
    CROSS JOIN advanced
), updated AS (
    UPDATE tracks AS current
    SET storage_state = CASE WHEN decision.too_long THEN 'too_long' ELSE 'ok' END,
        storage_quality = CASE
            WHEN decision.too_long THEN current.storage_quality
            ELSE COALESCE($6::text, current.storage_quality)
        END,
        storage_attempts = CASE WHEN decision.too_long THEN current.storage_attempts ELSE 0 END,
        s3_verified_at = CASE WHEN decision.too_long THEN current.s3_verified_at ELSE now() END,
        s3_missing_at = CASE WHEN decision.too_long THEN current.s3_missing_at ELSE NULL END,
        hq_upgrade_pending = CASE
            WHEN decision.too_long THEN false
            WHEN $6::text = 'hq' THEN false
            WHEN $6::text = 'sq' AND current.storage_quality IS DISTINCT FROM 'hq' THEN true
            ELSE current.hq_upgrade_pending
        END,
        index_state = CASE
            WHEN decision.too_long THEN 'too_long'
            WHEN decision.reindex THEN 'pending'
            ELSE current.index_state
        END,
        indexed_at = CASE
            WHEN decision.too_long OR decision.reindex THEN NULL
            ELSE current.indexed_at
        END,
        transcribe_state = CASE
            WHEN decision.too_long THEN 'disabled'
            WHEN EXISTS (SELECT 1 FROM quarantined) THEN 'quarantined'
            ELSE current.transcribe_state
        END,
        transcribe_at = CASE
            WHEN decision.too_long OR EXISTS (SELECT 1 FROM quarantined) THEN now()
            ELSE current.transcribe_at
        END,
        updated_at = now()
    FROM decision
    WHERE current.sc_track_id = $5
    RETURNING decision.uploaded_generation,
              decision.too_long,
              decision.dispatch_audio AND NOT decision.too_long AS dispatch_audio,
              decision.dispatch_transcription AND NOT decision.too_long AS dispatch_transcription
)
SELECT EXISTS (SELECT 1 FROM track) AS "known!",
       EXISTS (SELECT 1 FROM accepted) AS "accepted!",
       EXISTS (SELECT 1 FROM advanced) AS "advanced!",
       EXISTS (SELECT 1 FROM updated) AS "updated!",
       (SELECT uploaded_generation FROM updated) AS uploaded_generation,
       COALESCE((SELECT too_long FROM updated), false) AS "too_long!",
       COALESCE((SELECT dispatch_audio FROM updated), false) AS "dispatch_audio!",
       COALESCE((SELECT dispatch_transcription FROM updated), false) AS "dispatch_transcription!"
