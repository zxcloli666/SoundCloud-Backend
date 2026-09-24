WITH lyrics AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.created_at,
           cache.content_generation,
           cache.embedded_at,
           cache.embedding_state
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id = $5
    FOR UPDATE
), wire AS MATERIALIZED (
    SELECT state.sc_track_id,
           state.status,
           state.lyrics_created_at,
           state.lyrics_content_generation,
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
), owned AS MATERIALIZED (
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
), eligible AS MATERIALIZED (
    SELECT owned.sc_track_id,
           CASE
               WHEN $7::text = 'reopenable' AND owned.reopen_count >= $10::integer
                   THEN 'failed'
               ELSE $7::text
           END AS status
    FROM owned
    JOIN wire ON wire.sc_track_id = owned.sc_track_id
    JOIN lyrics ON lyrics.sc_track_id = owned.sc_track_id
    WHERE lyrics.created_at = wire.lyrics_created_at
      AND lyrics.content_generation = wire.lyrics_content_generation
      AND lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IN ('pending', 'dispatched')
), cache_settled AS (
    UPDATE lyrics_cache AS cache
    SET embedded_at = CASE WHEN eligible.status = 'done' THEN now() ELSE NULL END,
        embedding_state = CASE eligible.status
            WHEN 'done' THEN 'done'
            WHEN 'skipped' THEN 'skipped'
            WHEN 'failed' THEN 'failed'
            ELSE NULL
        END,
        language = CASE
            WHEN eligible.status IN ('done', 'skipped') THEN COALESCE($9::varchar, cache.language)
            ELSE cache.language
        END
    FROM eligible
    WHERE cache.sc_track_id = eligible.sc_track_id
    RETURNING cache.sc_track_id
), wire_settled AS (
    UPDATE lyrics_embedding_wire_state AS state
    SET status = eligible.status,
        result_kind = CASE eligible.status
            WHEN 'done' THEN 'vector'
            WHEN 'skipped' THEN 'skipped'
            WHEN 'failed' THEN 'failed'
            ELSE 'reopen'
        END,
        result_reason = $8::varchar,
        completed_at = now(),
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM eligible
    JOIN cache_settled
      ON cache_settled.sc_track_id = eligible.sc_track_id
    WHERE state.sc_track_id = eligible.sc_track_id
    RETURNING state.sc_track_id,
              state.status
), accepted AS (
    INSERT INTO pipeline_event_receipts (
        consumer,
        stream,
        stream_sequence,
        event_published_at
    )
    SELECT $1, $2, $3, $4
    FROM wire_settled
    ON CONFLICT DO NOTHING
    RETURNING 1
)
SELECT EXISTS (SELECT 1 FROM receipt) AS "already_settled!",
       EXISTS (SELECT 1 FROM owned) AS "owned!",
       EXISTS (SELECT 1 FROM eligible) AS "lyrics_current!",
       EXISTS (SELECT 1 FROM accepted) AS "accepted!",
       (SELECT status FROM wire_settled) AS settled_status
