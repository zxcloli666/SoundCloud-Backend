WITH track AS MATERIALIZED (
    SELECT current.sc_track_id,
           current.storage_state,
           current.index_state,
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
      AND track.index_state NOT IN ('indexed', 'too_long')
      AND NOT track.needs_duration_resolve
    FOR UPDATE OF event
), claimed AS (
    INSERT INTO audio_index_wire_state (
        sc_track_id,
        status,
        upload_generation,
        attempt,
        dispatched_at
    )
    SELECT storage.sc_track_id, 'pending', storage.uploaded_generation, 1, now()
    FROM storage
    ON CONFLICT (sc_track_id) DO UPDATE
    SET status = 'pending',
        upload_generation = EXCLUDED.upload_generation,
        attempt = CASE
            WHEN audio_index_wire_state.upload_generation IS DISTINCT FROM EXCLUDED.upload_generation
                THEN 1
            WHEN audio_index_wire_state.status = 'quarantined'
                THEN audio_index_wire_state.attempt + 1
            ELSE audio_index_wire_state.attempt
        END,
        outcome_rank = 0,
        outcome_status = NULL,
        outcome_reason = NULL,
        dispatched_at = now(),
        completed_at = NULL,
        quarantine_reason = NULL,
        result_consumer = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_consumer
        END,
        result_stream = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_stream
        END,
        result_stream_sequence = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_stream_sequence
        END,
        result_published_at = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_published_at
        END,
        result_lease_id = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_lease_id
        END,
        result_lease_expires_at = CASE
            WHEN audio_index_wire_state.result_lease_expires_at > now()
                THEN audio_index_wire_state.result_lease_expires_at
        END,
        updated_at = now()
    WHERE audio_index_wire_state.upload_generation IS DISTINCT FROM EXCLUDED.upload_generation
       OR audio_index_wire_state.status IN ('pending', 'quarantined')
       OR (
           audio_index_wire_state.status = 'reopenable'
           AND audio_index_wire_state.outcome_reason = 'dispatch_publish_failed'
       )
    RETURNING attempt
)
SELECT (SELECT attempt FROM claimed) AS attempt
