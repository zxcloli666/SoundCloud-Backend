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
), owned AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM wire
    JOIN track ON track.sc_track_id = wire.sc_track_id
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND wire.status IN ('pending', 'terminal', 'reopenable')
      AND wire.upload_generation = $6
      AND wire.attempt = $8
      AND wire.result_lease_id = $7
      AND wire.result_lease_expires_at > now()
      AND track.storage_state = 'ok'
      AND track.index_state NOT IN ('indexed', 'too_long')
      AND NOT track.needs_duration_resolve
), committed AS (
    UPDATE audio_index_wire_state AS state
    SET status = 'done',
        outcome_rank = 4,
        outcome_status = 'ok',
        outcome_reason = NULL,
        completed_at = now(),
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM owned
    WHERE state.sc_track_id = owned.sc_track_id
    RETURNING state.sc_track_id
), indexed AS (
    UPDATE tracks AS current
    SET index_state = 'indexed',
        indexed_at = now(),
        index_attempts = 0,
        updated_at = now()
    FROM committed
    WHERE current.sc_track_id = committed.sc_track_id
    RETURNING current.sc_track_id
), invalidating AS MATERIALIZED (
    SELECT track.sc_track_id
    FROM track
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND NOT EXISTS (SELECT 1 FROM owned)
      AND NOT EXISTS (
          SELECT 1
          FROM wire
          WHERE wire.status = 'pending'
      )
      AND NOT EXISTS (
          SELECT 1
          FROM wire
          WHERE wire.status = 'done'
            AND wire.upload_generation = $6
      )
      AND NOT EXISTS (
          SELECT 1
          FROM wire
          WHERE wire.status IN ('terminal', 'reopenable')
            AND wire.upload_generation = $6
            AND wire.attempt = $8
            AND (
                wire.result_lease_id = $7
                OR wire.result_lease_id IS NULL
                OR wire.result_lease_expires_at <= now()
            )
      )
), demoted AS (
    UPDATE tracks AS current
    SET index_state = 'pending',
        indexed_at = NULL,
        updated_at = now()
    FROM invalidating
    WHERE current.sc_track_id = invalidating.sc_track_id
      AND current.index_state <> 'too_long'
    RETURNING current.sc_track_id
), released AS (
    UPDATE audio_index_wire_state AS state
    SET status = CASE
            WHEN EXISTS (SELECT 1 FROM invalidating) THEN 'quarantined'
            ELSE state.status
        END,
        quarantine_reason = CASE
            WHEN EXISTS (SELECT 1 FROM invalidating)
                THEN 'superseded_result_overwrote_point'
            ELSE state.quarantine_reason
        END,
        completed_at = CASE
            WHEN EXISTS (SELECT 1 FROM invalidating) THEN now()
            ELSE state.completed_at
        END,
        result_lease_id = CASE
            WHEN state.result_lease_id = $7 THEN NULL
            ELSE state.result_lease_id
        END,
        result_lease_expires_at = CASE
            WHEN state.result_lease_id = $7 THEN NULL
            ELSE state.result_lease_expires_at
        END,
        updated_at = now()
    FROM wire
    WHERE state.sc_track_id = wire.sc_track_id
      AND NOT EXISTS (SELECT 1 FROM receipt)
      AND NOT EXISTS (SELECT 1 FROM owned)
      AND (
          state.result_lease_id = $7
          OR EXISTS (SELECT 1 FROM invalidating)
      )
    RETURNING state.sc_track_id
), accepted AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM track
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND (
          EXISTS (SELECT 1 FROM committed)
          OR EXISTS (SELECT 1 FROM invalidating)
      )
    ON CONFLICT DO NOTHING
    RETURNING 1
)
SELECT EXISTS (SELECT 1 FROM receipt) AS "already_settled!",
       EXISTS (SELECT 1 FROM committed) AS "committed!",
       EXISTS (SELECT 1 FROM indexed) AS "indexed!",
       EXISTS (SELECT 1 FROM invalidating) AS "invalidated!",
       EXISTS (SELECT 1 FROM demoted) AS "demoted!",
       EXISTS (SELECT 1 FROM released) AS "released!",
       EXISTS (SELECT 1 FROM accepted) AS "accepted!"
