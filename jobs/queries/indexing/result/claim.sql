WITH track AS MATERIALIZED (
    SELECT current.sc_track_id,
           current.storage_state,
           current.index_state,
           current.needs_duration_resolve
    FROM tracks AS current
    WHERE current.sc_track_id = $5
    FOR UPDATE
), wire AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.upload_generation,
           state.attempt,
           state.result_consumer,
           state.result_stream,
           state.result_stream_sequence,
           state.result_published_at,
           state.result_lease_id,
           state.result_lease_expires_at
    FROM audio_index_wire_state AS state
    JOIN track ON track.sc_track_id = state.sc_track_id
    FOR UPDATE OF state
), receipt AS MATERIALIZED (
    SELECT 1
    FROM pipeline_event_receipts
    WHERE consumer = $1
      AND stream = $2
      AND stream_sequence = $3
      AND event_published_at = $4
), correlated AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM wire
    JOIN track ON track.sc_track_id = wire.sc_track_id
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND wire.status IN ('pending', 'terminal', 'reopenable')
      AND wire.upload_generation = $6
      AND wire.attempt = $9
      AND track.index_state NOT IN ('indexed', 'too_long')
), deferred AS MATERIALIZED (
    SELECT correlated.sc_track_id
    FROM correlated
    JOIN track ON track.sc_track_id = correlated.sc_track_id
    WHERE track.storage_state <> 'ok'
       OR track.needs_duration_resolve
), eligible AS MATERIALIZED (
    SELECT correlated.sc_track_id
    FROM correlated
    WHERE NOT EXISTS (SELECT 1 FROM deferred)
), claimed AS (
    UPDATE audio_index_wire_state AS state
    SET result_consumer = $1,
        result_stream = $2,
        result_stream_sequence = $3,
        result_published_at = $4,
        result_lease_id = $7,
        result_lease_expires_at = now() + $8::bigint * interval '1 second',
        updated_at = now()
    FROM eligible
    JOIN wire ON wire.sc_track_id = eligible.sc_track_id
    WHERE state.sc_track_id = eligible.sc_track_id
      AND (
          wire.result_lease_id IS NULL
          OR wire.result_lease_expires_at <= now()
          OR (
              wire.result_consumer = $1
              AND wire.result_stream = $2
              AND wire.result_stream_sequence = $3
              AND wire.result_published_at = $4
          )
      )
    RETURNING state.sc_track_id
), postponed AS (
    UPDATE audio_index_wire_state AS state
    SET result_consumer = $1,
        result_stream = $2,
        result_stream_sequence = $3,
        result_published_at = $4,
        updated_at = now()
    FROM deferred
    WHERE state.sc_track_id = deferred.sc_track_id
    RETURNING state.sc_track_id
), invalidating AS MATERIALIZED (
    SELECT track.sc_track_id
    FROM track
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND NOT EXISTS (SELECT 1 FROM correlated)
      AND track.index_state = 'indexed'
      AND NOT EXISTS (
          SELECT 1
          FROM wire
          WHERE wire.upload_generation = $6
      )
), demoted AS (
    UPDATE tracks AS current
    SET index_state = 'pending',
        indexed_at = NULL,
        updated_at = now()
    FROM invalidating
    WHERE current.sc_track_id = invalidating.sc_track_id
    RETURNING current.sc_track_id
), quarantined AS (
    UPDATE audio_index_wire_state AS state
    SET status = 'quarantined',
        quarantine_reason = 'superseded_result_overwrote_point',
        completed_at = now(),
        updated_at = now()
    FROM demoted
    JOIN wire ON wire.sc_track_id = demoted.sc_track_id
    WHERE state.sc_track_id = demoted.sc_track_id
      AND wire.status <> 'pending'
    RETURNING state.sc_track_id
), dismissed AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM track
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND NOT EXISTS (SELECT 1 FROM correlated)
    ON CONFLICT DO NOTHING
    RETURNING 1
)
SELECT EXISTS (SELECT 1 FROM track) AS "known!",
       NOT EXISTS (SELECT 1 FROM correlated)
           AND EXISTS (SELECT 1 FROM track) AS "settled!",
       EXISTS (SELECT 1 FROM claimed) AS "claimed!",
       EXISTS (SELECT 1 FROM demoted) AS "demoted!",
       EXISTS (SELECT 1 FROM deferred) AS "deferred!",
       EXISTS (SELECT 1 FROM eligible)
           AND NOT EXISTS (SELECT 1 FROM claimed) AS "busy!"
