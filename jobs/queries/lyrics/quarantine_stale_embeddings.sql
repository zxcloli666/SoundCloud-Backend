WITH candidates AS MATERIALIZED (
    SELECT lyrics.sc_track_id,
           lyrics.embedding_state IN ('pending', 'dispatched')
               AND lyrics.created_at = wire.lyrics_created_at
               AND lyrics.content_generation IS NOT DISTINCT FROM wire.lyrics_content_generation
               AS current
    FROM lyrics_embedding_wire_state AS wire
    JOIN lyrics_cache AS lyrics
      ON lyrics.sc_track_id = wire.sc_track_id
    WHERE wire.status = 'pending'
      AND CASE
          WHEN wire.result_consumer IS NULL
              THEN wire.first_publish_attempt_at
          ELSE wire.updated_at
      END < now() - $2::bigint * interval '1 second'
      AND (
          wire.result_lease_id IS NULL
          OR wire.result_lease_expires_at <= now()
      )
    ORDER BY wire.first_publish_attempt_at, wire.sc_track_id
    FOR UPDATE OF lyrics SKIP LOCKED
    LIMIT $1
), locked_wire AS MATERIALIZED (
    SELECT wire.sc_track_id,
           candidates.current
    FROM lyrics_embedding_wire_state AS wire
    JOIN candidates
      ON candidates.sc_track_id = wire.sc_track_id
    WHERE wire.status = 'pending'
    FOR UPDATE OF wire
), quarantined AS (
    UPDATE lyrics_embedding_wire_state AS wire
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = CASE
            WHEN locked_wire.current THEN 'result_timeout'
            ELSE 'lyrics_changed_before_apply'
        END,
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM locked_wire
    WHERE wire.sc_track_id = locked_wire.sc_track_id
    RETURNING wire.sc_track_id,
              locked_wire.current
), released AS (
    UPDATE lyrics_cache AS lyrics
    SET embedding_state = CASE
            WHEN quarantined.current THEN 'quarantined'
            ELSE NULL
        END
    FROM quarantined
    WHERE lyrics.sc_track_id = quarantined.sc_track_id
      AND lyrics.embedding_state IN ('pending', 'dispatched')
    RETURNING lyrics.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
