WITH lyrics AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.created_at,
           cache.content_generation,
           cache.embedded_at IS NULL
               AND cache.embedding_state IN ('pending', 'dispatched') AS awaiting
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id = $1
    FOR UPDATE
), request AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.reopen_count,
           COALESCE(
               lyrics.awaiting
                   AND state.lyrics_created_at = lyrics.created_at
                   AND state.lyrics_content_generation = lyrics.content_generation,
               false
           ) AS current
    FROM lyrics_embedding_wire_state AS state
    LEFT JOIN lyrics
      ON lyrics.sc_track_id = state.sc_track_id
    WHERE state.sc_track_id = $1
      AND state.status = 'pending'
      AND state.request_message_id = $2
      AND state.result_consumer IS NULL
    FOR UPDATE OF state
), eligible AS MATERIALIZED (
    SELECT request.sc_track_id,
           request.current,
           CASE
               WHEN NOT request.current THEN 'quarantined'
               WHEN request.reopen_count >= $4::integer THEN 'failed'
               ELSE 'reopenable'
           END AS status
    FROM request
), cache_settled AS (
    UPDATE lyrics_cache AS cache
    SET embedding_state = CASE eligible.status WHEN 'failed' THEN 'failed' ELSE NULL END
    FROM eligible
    WHERE cache.sc_track_id = eligible.sc_track_id
      AND eligible.current
    RETURNING cache.sc_track_id
), wire_settled AS (
    UPDATE lyrics_embedding_wire_state AS state
    SET status = eligible.status,
        result_reason = $3::varchar,
        quarantine_reason = CASE
            WHEN eligible.current THEN NULL
            ELSE 'lyrics_changed_before_apply'
        END,
        completed_at = now(),
        updated_at = now()
    FROM eligible
    WHERE state.sc_track_id = eligible.sc_track_id
      AND (
          NOT eligible.current
          OR EXISTS (SELECT 1 FROM cache_settled)
      )
    RETURNING state.status
)
SELECT status AS "status!"
FROM wire_settled
