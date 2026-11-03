WITH owned_state AS MATERIALIZED (
    SELECT state.track_id,
           state.sc_track_id,
           ($9::boolean AND state.failure_streak < $10::integer) AS recheck
    FROM tracks AS track
    JOIN lyrics_lookup_state AS state ON state.track_id = track.id
    JOIN background_jobs AS job
      ON job.id = $1
     AND job.generation = $2
     AND job.lease_generation = $2
     AND job.lease_id = $3
     AND job.lease_expires_at > now()
    WHERE state.track_id = $4
      AND state.generation = $5
      AND state.claim_job_id = $1
      AND state.claim_job_generation = $2
      AND state.claim_job_lease_id = $3
      AND state.claim_state_generation = $5
      AND state.claim_expires_at > now()
    FOR UPDATE OF track, state
), upserted AS (
    INSERT INTO lyrics_cache (
        sc_track_id,
        track_id,
        synced_lrc,
        plain_text,
        source,
        plain_source,
        synced_source,
        language,
        language_confidence,
        embedded_at,
        embedding_state,
        content_generation,
        updated_at
    )
    SELECT owned_state.sc_track_id,
           owned_state.track_id,
           NULLIF($7::text, ''),
           NULLIF($8::text, ''),
           $6::varchar(16),
           CASE WHEN NULLIF($8::text, '') IS NULL THEN NULL ELSE $6::varchar(16) END,
           CASE WHEN NULLIF($7::text, '') IS NULL THEN NULL ELSE $6::varchar(16) END,
           NULL,
           NULL,
           NULL,
           NULL,
           1,
           now()
    FROM owned_state
    WHERE NULLIF($7::text, '') IS NOT NULL
       OR NULLIF($8::text, '') IS NOT NULL
    ON CONFLICT (sc_track_id) DO UPDATE
    SET track_id = EXCLUDED.track_id,
        synced_lrc = COALESCE(lyrics_cache.synced_lrc, EXCLUDED.synced_lrc),
        plain_text = CASE
            WHEN lyrics_cache.source NOT IN ('lrclib', 'musixmatch', 'genius', 'netease')
                 AND EXCLUDED.plain_text IS NOT NULL
                THEN EXCLUDED.plain_text
            ELSE COALESCE(lyrics_cache.plain_text, EXCLUDED.plain_text)
        END,
        source = CASE
            WHEN lyrics_cache.source IN ('lrclib', 'musixmatch', 'genius', 'netease')
                THEN lyrics_cache.source
            ELSE EXCLUDED.source
        END,
        plain_source = CASE
            WHEN lyrics_cache.source NOT IN ('lrclib', 'musixmatch', 'genius', 'netease')
                 AND EXCLUDED.plain_text IS NOT NULL
                THEN EXCLUDED.plain_source
            WHEN lyrics_cache.plain_text IS NOT NULL
                THEN COALESCE(lyrics_cache.plain_source, lyrics_cache.source)
            ELSE EXCLUDED.plain_source
        END,
        synced_source = CASE
            WHEN lyrics_cache.synced_lrc IS NOT NULL
                THEN COALESCE(lyrics_cache.synced_source, lyrics_cache.source)
            ELSE EXCLUDED.synced_source
        END,
        synced_version = CASE
            WHEN lyrics_cache.synced_lrc IS NOT NULL
                THEN lyrics_cache.synced_version
            ELSE NULL
        END,
        embedded_at = NULL,
        embedding_state = NULL,
        content_generation = lyrics_cache.content_generation + 1,
        updated_at = now()
    WHERE NOT $9::boolean
       OR lyrics_cache.plain_text IS NULL
       OR lyrics_cache.source NOT IN ('lrclib', 'musixmatch', 'genius', 'netease')
    RETURNING lyrics_cache.sc_track_id,
              lyrics_cache.synced_lrc,
              lyrics_cache.plain_text,
              lyrics_cache.source
), rescheduled AS (
    UPDATE lyrics_lookup_state AS state
    SET status = 'retry',
        next_run_at = now() + interval '24 hours',
        failure_streak = state.failure_streak + 1,
        last_outcome = 'recheck',
        last_error = NULL,
        retry_after_at = NULL,
        wake_message_id = NULL,
        wake_generation = NULL,
        wake_durable_at = NULL,
        claim_job_id = NULL,
        claim_job_generation = NULL,
        claim_job_lease_id = NULL,
        claim_state_generation = NULL,
        claim_expires_at = NULL,
        updated_at = now()
    FROM owned_state
    WHERE state.track_id = owned_state.track_id
      AND owned_state.recheck
    RETURNING state.track_id
), removed AS (
    DELETE FROM lyrics_lookup_state AS state
    USING owned_state
    WHERE state.track_id = owned_state.track_id
      AND NOT owned_state.recheck
    RETURNING state.track_id
)
SELECT owned_state.sc_track_id AS "sc_track_id!",
       upserted.synced_lrc,
       upserted.plain_text,
       upserted.source,
       (EXISTS (SELECT 1 FROM removed) OR EXISTS (SELECT 1 FROM rescheduled)) AS "settled!"
FROM owned_state
         LEFT JOIN upserted ON true
