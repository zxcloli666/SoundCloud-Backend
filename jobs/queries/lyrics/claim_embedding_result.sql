WITH wire AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.request_message_id,
           state.request_sha256,
           state.lyrics_created_at,
           state.lyrics_content_generation,
           state.result_consumer,
           state.result_stream,
           state.result_kind,
           state.result_stream_sequence,
           state.result_published_at,
           state.result_rank,
           state.result_lease_id,
           state.result_lease_expires_at
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
    SELECT wire.*,
           COALESCE(
               wire.result_consumer = $1
               AND wire.result_stream = $2
               AND wire.result_stream_sequence = $3
               AND wire.result_published_at = $4,
               false
           ) AS same_delivery
    FROM wire
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND wire.request_message_id = $9
      AND (
          wire.status = 'pending'
          OR (
              wire.status IN ('skipped', 'failed', 'reopenable')
              AND COALESCE(wire.result_rank, 0) < $10::smallint
              AND EXISTS (
                  SELECT 1
                  FROM lyrics_cache AS cache
                  WHERE cache.sc_track_id = wire.sc_track_id
                    AND cache.created_at = wire.lyrics_created_at
                    AND cache.content_generation = wire.lyrics_content_generation
                    AND cache.embedded_at IS NULL
                    AND (
                        cache.embedding_state IS NULL
                        OR cache.embedding_state IN ('skipped', 'failed')
                    )
              )
          )
      )
), claimed AS (
    UPDATE lyrics_embedding_wire_state AS state
    SET status = 'pending',
        completed_at = NULL,
        result_consumer = $1,
        result_stream = $2,
        result_kind = $6,
        result_stream_sequence = $3,
        result_published_at = $4,
        result_rank = $10::smallint,
        result_lease_id = $7,
        result_lease_expires_at = now() + $8::bigint * interval '1 second',
        updated_at = now()
    FROM eligible
    WHERE state.sc_track_id = eligible.sc_track_id
      AND (
          state.result_consumer IS NULL
          OR (
              eligible.same_delivery
              AND state.result_kind = $6
              AND (
                  state.result_lease_id IS NULL
                  OR state.result_lease_expires_at <= now()
              )
          )
          OR (
              NOT eligible.same_delivery
              AND COALESCE(state.result_rank, 0) < $10::smallint
          )
      )
    RETURNING state.sc_track_id,
              state.result_kind
), reclaimed_lyrics AS (
    UPDATE lyrics_cache AS cache
    SET embedding_state = 'dispatched'
    FROM claimed
    JOIN eligible ON eligible.sc_track_id = claimed.sc_track_id
    WHERE cache.sc_track_id = claimed.sc_track_id
      AND eligible.status <> 'pending'
      AND cache.created_at = eligible.lyrics_created_at
      AND cache.content_generation = eligible.lyrics_content_generation
      AND cache.embedded_at IS NULL
      AND (
          cache.embedding_state IS NULL
          OR cache.embedding_state IN ('skipped', 'failed')
      )
    RETURNING cache.sc_track_id
), dismissed AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    WHERE NOT EXISTS (SELECT 1 FROM receipt)
      AND NOT EXISTS (SELECT 1 FROM claimed)
      AND NOT EXISTS (SELECT 1 FROM eligible WHERE eligible.same_delivery)
    ON CONFLICT DO NOTHING
    RETURNING 1
), lyrics AS MATERIALIZED (
    SELECT cache.plain_text,
           cache.synced_lrc
    FROM lyrics_cache AS cache
    JOIN eligible
      ON eligible.sc_track_id = cache.sc_track_id
    WHERE cache.created_at = eligible.lyrics_created_at
      AND cache.content_generation = eligible.lyrics_content_generation
      AND cache.embedded_at IS NULL
      AND (
          cache.embedding_state IN ('pending', 'dispatched')
          OR (
              eligible.status <> 'pending'
              AND (
                  cache.embedding_state IS NULL
                  OR cache.embedding_state IN ('skipped', 'failed')
              )
          )
      )
)
SELECT EXISTS (SELECT 1 FROM receipt) OR EXISTS (SELECT 1 FROM dismissed) AS "settled!",
       EXISTS (SELECT 1 FROM claimed) AS "claimed!",
       EXISTS (
           SELECT 1
           FROM eligible
           WHERE eligible.same_delivery
             AND eligible.result_kind = $6
             AND eligible.result_lease_id IS NOT NULL
             AND eligible.result_lease_expires_at > now()
       ) AS "busy!",
       EXISTS (
           SELECT 1
           FROM eligible
           WHERE eligible.same_delivery
             AND eligible.result_kind IS DISTINCT FROM $6::varchar
       ) AS "kind_mismatch!",
       (SELECT result_kind FROM claimed) AS result_kind,
       (SELECT request_sha256 FROM eligible) AS request_sha256,
       EXISTS (SELECT 1 FROM lyrics) AS "lyrics_current!",
       (SELECT plain_text FROM lyrics) AS plain_text,
       (SELECT synced_lrc FROM lyrics) AS synced_lrc
