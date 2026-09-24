WITH wire AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.reopen_count,
           state.result_consumer,
           state.result_stream,
           state.result_stream_sequence,
           state.result_published_at,
           state.result_lease_id
    FROM lyrics_embedding_wire_state AS state
    WHERE state.sc_track_id = $5
    FOR UPDATE
), receipt AS MATERIALIZED (
    SELECT 1
    FROM pipeline_event_receipts
    WHERE consumer = $1
      AND stream = $2
      AND stream_sequence = $3
      AND event_published_at = $4
), eligible AS MATERIALIZED (
    SELECT wire.sc_track_id,
           wire.reopen_count
    FROM wire
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND wire.status = 'pending'
      AND wire.result_consumer = $1
      AND wire.result_stream = $2
      AND wire.result_stream_sequence = $3
      AND wire.result_published_at = $4
      AND wire.result_lease_id = $6
), wire_quarantined AS (
    UPDATE lyrics_embedding_wire_state AS state
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = $7::varchar,
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM eligible
    WHERE state.sc_track_id = eligible.sc_track_id
    RETURNING state.sc_track_id
), cache_released AS (
    UPDATE lyrics_cache AS cache
    SET embedded_at = NULL,
        embedding_state = CASE
            WHEN eligible.reopen_count < $8::integer THEN NULL
            ELSE 'quarantined'
        END
    FROM eligible
    JOIN wire_quarantined
      ON wire_quarantined.sc_track_id = eligible.sc_track_id
    WHERE cache.sc_track_id = eligible.sc_track_id
      AND cache.embedding_state IN ('pending', 'dispatched')
    RETURNING cache.sc_track_id
), accepted AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM wire_quarantined
    ON CONFLICT DO NOTHING
    RETURNING 1
)
SELECT EXISTS (SELECT 1 FROM receipt) AS "already_settled!",
       EXISTS (SELECT 1 FROM eligible) AS "owned!",
       EXISTS (SELECT 1 FROM wire_quarantined) AS "wire_quarantined!",
       EXISTS (SELECT 1 FROM cache_released) AS "cache_released!",
       EXISTS (SELECT 1 FROM accepted) AS "accepted!"
