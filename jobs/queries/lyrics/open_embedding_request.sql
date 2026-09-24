WITH lyrics AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.created_at,
           cache.content_generation
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id = $1
      AND cache.content_generation = $2
      AND cache.embedded_at IS NULL
      AND cache.embedding_state = 'queued'
    FOR UPDATE
), opened AS (
    INSERT INTO lyrics_embedding_wire_state (
        sc_track_id,
        status,
        lyrics_created_at,
        lyrics_content_generation,
        request_version,
        request_text,
        request_language,
        request_sha256,
        request_message_id,
        first_publish_attempt_at,
        publish_retry_until
    )
    SELECT lyrics.sc_track_id,
           'pending',
           lyrics.created_at,
           lyrics.content_generation,
           1,
           $3::text,
           $4::varchar,
           $5::bytea,
           $6::varchar,
           now(),
           now() + interval '1 day'
    FROM lyrics
    ON CONFLICT (sc_track_id) DO UPDATE
    SET status = 'pending',
        lyrics_created_at = EXCLUDED.lyrics_created_at,
        reopen_count = CASE
            WHEN lyrics_embedding_wire_state.status IN ('reopenable', 'quarantined')
                 AND lyrics_embedding_wire_state.lyrics_content_generation
                     = EXCLUDED.lyrics_content_generation
                THEN lyrics_embedding_wire_state.reopen_count + 1
            ELSE 0
        END,
        lyrics_content_generation = EXCLUDED.lyrics_content_generation,
        request_version = EXCLUDED.request_version,
        request_text = EXCLUDED.request_text,
        request_language = EXCLUDED.request_language,
        request_sha256 = EXCLUDED.request_sha256,
        request_message_id = EXCLUDED.request_message_id,
        first_publish_attempt_at = EXCLUDED.first_publish_attempt_at,
        publish_retry_until = EXCLUDED.publish_retry_until,
        publish_acknowledged_at = NULL,
        completed_at = NULL,
        quarantine_reason = NULL,
        result_consumer = NULL,
        result_stream = NULL,
        result_kind = NULL,
        result_reason = NULL,
        result_rank = NULL,
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        result_stream_sequence = NULL,
        result_published_at = NULL,
        updated_at = now()
    WHERE lyrics_embedding_wire_state.status <> 'pending'
    RETURNING sc_track_id
), marked AS (
    UPDATE lyrics_cache AS cache
    SET embedding_state = 'pending'
    FROM opened
    WHERE cache.sc_track_id = opened.sc_track_id
    RETURNING cache.sc_track_id
)
SELECT EXISTS (SELECT 1 FROM marked) AS "opened!"
