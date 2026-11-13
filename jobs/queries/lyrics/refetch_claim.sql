WITH owned_job AS MATERIALIZED (
    SELECT id
    FROM background_jobs
    WHERE id = $1
      AND kind = 'lyrics.lookup_sweep'
      AND generation = $2
      AND lease_generation = $2
      AND lease_id = $3
      AND lease_expires_at > now()
), candidates AS MATERIALIZED (
    SELECT repair.sc_track_id
    FROM lyrics_repair AS repair
    JOIN owned_job ON true
    WHERE repair.kind = 'refetch'
      AND repair.status = 'pending'
      AND repair.claim_job_id IS NULL
      AND repair.next_run_at <= now()
    ORDER BY repair.priority DESC, repair.next_run_at, repair.sc_track_id
    FOR UPDATE OF repair SKIP LOCKED
    LIMIT $4
), claimed AS (
    UPDATE lyrics_repair AS repair
    SET claim_job_id = $1,
        claim_job_generation = $2,
        claim_job_lease_id = $3,
        claim_expires_at = now() + $5::bigint * interval '1 second',
        attempts = repair.attempts + 1,
        updated_at = now()
    FROM candidates
    WHERE repair.sc_track_id = candidates.sc_track_id
    RETURNING repair.sc_track_id, repair.attempts
)
SELECT claimed.sc_track_id AS "sc_track_id!",
       claimed.attempts AS "attempts!",
       btrim(track.title) AS input_title,
       CASE WHEN track.id IS NULL THEN NULL ELSE COALESCE(
           NULLIF(btrim(track.metadata_artist), ''),
           NULLIF(btrim(track.uploader_username), ''),
           ''
       ) END AS input_artist,
       track.duration_ms AS input_duration_ms,
       track.genius_song_id AS input_genius_song_id,
       NULLIF(btrim(track.genius_url), '') AS input_genius_url
FROM claimed
         LEFT JOIN tracks AS track ON track.sc_track_id = claimed.sc_track_id
