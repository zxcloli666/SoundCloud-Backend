WITH settled AS (
    UPDATE audio_index_wire_state AS state
    SET status = $4,
        outcome_rank = $5,
        outcome_status = $6,
        outcome_reason = $7,
        completed_at = now(),
        updated_at = now()
    WHERE state.sc_track_id = $1
      AND state.upload_generation = $2
      AND state.attempt = $3
      AND state.status IN ('pending', 'terminal', 'reopenable')
      AND state.outcome_rank < $5
    RETURNING state.sc_track_id,
              state.upload_generation,
              state.status,
              state.outcome_reason
), failed AS (
    UPDATE tracks AS track
    SET index_state = 'failed',
        updated_at = now()
    FROM settled
    JOIN storage_event_state AS storage
      ON storage.sc_track_id = settled.sc_track_id
     AND storage.uploaded_generation = settled.upload_generation
    WHERE track.sc_track_id = settled.sc_track_id
      AND settled.status = 'terminal'
      AND settled.outcome_reason <> 'audio_forbidden'
      AND track.index_state = 'pending'
    RETURNING track.sc_track_id
)
SELECT settled.sc_track_id AS "sc_track_id!"
FROM settled
