WITH owned_state AS MATERIALIZED (
    SELECT state.track_id,
           state.sc_track_id,
           state.generation = $5 AS current,
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
    FOR UPDATE OF track, state
), claim AS MATERIALIZED (
    SELECT owned_state.track_id,
           owned_state.sc_track_id,
           owned_state.current,
           owned_state.recheck
    FROM owned_state
    JOIN lyrics_lookup_queue AS queue
      ON queue.track_id = owned_state.track_id
    WHERE queue.claim_job_id = $1
      AND queue.claim_job_generation = $2
      AND queue.claim_job_lease_id = $3
      AND queue.claim_state_generation = $5
      AND queue.claim_expires_at > now()
    FOR UPDATE OF queue
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
    SELECT claim.sc_track_id,
           claim.track_id,
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
    FROM claim
    WHERE claim.current
      AND (NULLIF($7::text, '') IS NOT NULL OR NULLIF($8::text, '') IS NOT NULL)
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
        updated_at = now()
    FROM claim
    WHERE state.track_id = claim.track_id
      AND claim.current
      AND claim.recheck
    RETURNING state.track_id, state.priority, state.next_run_at
), removed AS (
    DELETE FROM lyrics_lookup_state AS state
    USING claim
    WHERE state.track_id = claim.track_id
      AND claim.current
      AND NOT claim.recheck
    RETURNING state.track_id
), released AS (
    UPDATE lyrics_lookup_queue AS queue
    SET priority = COALESCE(rescheduled.priority, queue.priority),
        next_run_at = COALESCE(rescheduled.next_run_at, queue.next_run_at),
        wake_at = CASE WHEN rescheduled.track_id IS NULL THEN queue.wake_at END,
        claim_job_id = NULL,
        claim_job_generation = NULL,
        claim_job_lease_id = NULL,
        claim_state_generation = NULL,
        claim_expires_at = NULL
    FROM claim
    LEFT JOIN rescheduled ON rescheduled.track_id = claim.track_id
    WHERE queue.track_id = claim.track_id
      AND (rescheduled.track_id IS NOT NULL OR NOT claim.current)
    RETURNING queue.track_id
)
SELECT claim.sc_track_id AS "sc_track_id!",
       upserted.synced_lrc,
       upserted.plain_text,
       upserted.source,
       (EXISTS (SELECT 1 FROM removed) OR EXISTS (SELECT 1 FROM rescheduled)) AS "settled!"
FROM claim
         LEFT JOIN upserted ON true
WHERE claim.current
