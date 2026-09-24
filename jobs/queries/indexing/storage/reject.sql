WITH track AS MATERIALIZED (
    SELECT sc_track_id
    FROM tracks
    WHERE sc_track_id = $5
    FOR UPDATE
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
    SELECT 1
    FROM accepted
    WHERE NOT EXISTS (
        SELECT 1
        FROM storage_event_state AS state
        WHERE state.sc_track_id = $5
          AND (state.event_published_at, state.stream_sequence) >= ($4, $3)
    )
), updated AS (
    UPDATE tracks
    SET storage_attempts = storage_attempts + CASE
            WHEN storage_state IN ('pending', 'missing') THEN 1
            ELSE 0
        END,
        storage_state = CASE
            WHEN storage_state IN ('pending', 'missing')
                 AND storage_attempts + 1 >= $6
                THEN 'failed'
            ELSE storage_state
        END,
        hq_upgrade_pending = false,
        updated_at = now()
    WHERE sc_track_id = $5
      AND EXISTS (SELECT 1 FROM advanced)
    RETURNING 1
)
SELECT EXISTS (SELECT 1 FROM track) AS known,
       EXISTS (SELECT 1 FROM accepted) AS accepted,
       EXISTS (SELECT 1 FROM advanced) AS advanced,
       EXISTS (SELECT 1 FROM updated) AS updated
