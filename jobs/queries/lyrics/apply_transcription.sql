WITH track AS MATERIALIZED (
    SELECT current.sc_track_id,
           current.storage_state,
           current.needs_duration_resolve
    FROM tracks AS current
    WHERE current.sc_track_id = $5
    FOR UPDATE
), storage AS MATERIALIZED (
    SELECT event.sc_track_id,
           event.uploaded_generation,
           event.transcription_generation
    FROM storage_event_state AS event
    JOIN track ON track.sc_track_id = event.sc_track_id
    FOR UPDATE OF event
), transcription AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.upload_generation,
           state.attempt,
           state.result_rank
    FROM transcription_wire_state AS state
    JOIN storage ON storage.sc_track_id = state.sc_track_id
    FOR UPDATE OF state
), lyrics AS MATERIALIZED (
    SELECT cache.sc_track_id
    FROM lyrics_cache AS cache
    JOIN transcription ON transcription.sc_track_id = cache.sc_track_id
    FOR UPDATE OF cache
), correlated AS MATERIALIZED (
    SELECT track.sc_track_id,
           track.storage_state = 'ok' AND NOT track.needs_duration_resolve AS ready
    FROM track
    JOIN storage ON storage.sc_track_id = track.sc_track_id
    JOIN transcription ON transcription.sc_track_id = track.sc_track_id
    WHERE transcription.upload_generation = $6
      AND transcription.attempt = $7
      AND storage.uploaded_generation = $6
      AND storage.transcription_generation = $6
      AND COALESCE(transcription.result_rank, 0) < $11::smallint
      AND ($8::text <> 'done' OR EXISTS (SELECT 1 FROM lyrics))
), deferred AS MATERIALIZED (
    SELECT correlated.sc_track_id
    FROM correlated
    WHERE NOT correlated.ready
), accepted AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM track
    WHERE NOT EXISTS (SELECT 1 FROM deferred)
    ON CONFLICT DO NOTHING
    RETURNING 1
), eligible AS MATERIALIZED (
    SELECT correlated.sc_track_id
    FROM correlated
    CROSS JOIN accepted
    WHERE correlated.ready
), aligned AS (
    UPDATE lyrics_cache AS cache
    SET synced_lrc = CASE
            WHEN NULLIF(btrim(cache.synced_lrc), '') IS NULL THEN $13::text
            ELSE cache.synced_lrc
        END,
        synced_source = CASE
            WHEN NULLIF(btrim(cache.synced_lrc), '') IS NULL THEN 'self_gen'
            ELSE cache.synced_source
        END,
        synced_version = CASE
            WHEN NULLIF(btrim(cache.synced_lrc), '') IS NULL THEN $12::varchar
            ELSE cache.synced_version
        END,
        language = COALESCE(cache.language, $19::varchar)
    FROM eligible
    WHERE cache.sc_track_id = eligible.sc_track_id
      AND $8::text = 'done'
    RETURNING cache.sc_track_id
), settled AS (
    UPDATE transcription_wire_state AS state
    SET status = $8::text,
        reason = $10::varchar,
        quarantine_reason = CASE WHEN $8::text = 'quarantined' THEN $10::varchar ELSE NULL END,
        result_rank = $11::smallint,
        sync_version = $12::varchar,
        confidence = $14::float8,
        placed_share = $15::float8,
        aligned_share = $16::float8,
        lines_total = $17::integer,
        lines_unplaced = $18::integer,
        result_language = $19::varchar,
        completed_at = now(),
        result_stream_sequence = $3,
        result_published_at = $4,
        updated_at = now()
    FROM eligible
    WHERE state.sc_track_id = eligible.sc_track_id
    RETURNING state.sc_track_id
), seen AS (
    INSERT INTO transcription_sync_versions (sync_version)
    SELECT $12::varchar
    FROM settled
    WHERE $8::text IN ('done', 'rejected')
      AND $20::boolean
    ON CONFLICT (sync_version) DO UPDATE
    SET last_seen_at = now()
    WHERE transcription_sync_versions.last_seen_at < now() - interval '1 minute'
    RETURNING 1
), tracked AS (
    UPDATE tracks AS current
    SET transcribe_state = $9::text,
        transcribe_at = now(),
        updated_at = now()
    FROM settled
    WHERE current.sc_track_id = settled.sc_track_id
    RETURNING current.sc_track_id
)
SELECT EXISTS (SELECT 1 FROM track) AS "known!",
       EXISTS (SELECT 1 FROM deferred) AS "deferred!",
       EXISTS (SELECT 1 FROM accepted) AS "accepted!",
       EXISTS (SELECT 1 FROM settled) AS "applied!",
       EXISTS (SELECT 1 FROM aligned) AS "aligned!"
