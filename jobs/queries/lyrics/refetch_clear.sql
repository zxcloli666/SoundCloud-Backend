WITH owned AS MATERIALIZED (
    SELECT repair.sc_track_id
    FROM lyrics_repair AS repair
    JOIN background_jobs AS job
      ON job.id = $1
     AND job.generation = $2
     AND job.lease_generation = $2
     AND job.lease_id = $3
     AND job.lease_expires_at > now()
    WHERE repair.sc_track_id = $4
      AND repair.kind = 'refetch'
      AND repair.status = 'pending'
      AND repair.claim_job_id = $1
      AND repair.claim_job_generation = $2
      AND repair.claim_job_lease_id = $3
      AND repair.claim_expires_at > now()
    FOR UPDATE OF repair
), deleted AS (
    DELETE FROM lyrics_cache AS cache
    USING owned
    WHERE cache.sc_track_id = owned.sc_track_id
      AND cache.content_generation = $5
      AND NULLIF(btrim(cache.synced_lrc), '') IS NULL
    RETURNING cache.plain_text
), requeued AS (
    INSERT INTO lyrics_lookup_state (
        track_id,
        sc_track_id,
        priority,
        input_title,
        input_artist,
        input_duration_ms,
        input_genius_song_id,
        input_genius_url,
        input_release_date,
        input_sc_created_at,
        created_at
    )
    SELECT track.id,
           track.sc_track_id,
           track.index_priority,
           btrim(track.title),
           COALESCE(
               NULLIF(btrim(track.metadata_artist), ''),
               NULLIF(btrim(track.uploader_username), ''),
               ''
           ),
           track.duration_ms,
           track.genius_song_id,
           NULLIF(btrim(track.genius_url), ''),
           track.release_date,
           track.sc_created_at,
           track.created_at
    FROM tracks AS track
    WHERE track.sc_track_id = $4
      AND NOT track.pipeline_held
      AND EXISTS (SELECT 1 FROM deleted)
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_id
)
UPDATE lyrics_repair AS repair
SET status = 'done',
    outcome = CASE WHEN EXISTS (SELECT 1 FROM deleted) THEN 'cleared' ELSE 'stale' END,
    old_plain_text = (SELECT deleted.plain_text FROM deleted),
    claim_job_id = NULL,
    claim_job_generation = NULL,
    claim_job_lease_id = NULL,
    claim_expires_at = NULL,
    settled_at = now(),
    updated_at = now()
FROM owned
WHERE repair.sc_track_id = owned.sc_track_id
RETURNING repair.outcome AS "outcome!"
