SELECT EXISTS (
    SELECT 1
    FROM transcription_wire_state AS state
    WHERE state.sc_track_id = $1
      AND state.upload_generation = $2
      AND (
          state.status = 'pending'
          OR (
              state.status = 'reopenable'
              AND state.reason = 'dispatch_publish_failed'
          )
      )
) AS "claimed!"
