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
    SELECT state.track_id
    FROM lyrics_lookup_state AS state
    JOIN owned_job ON true
    WHERE state.next_run_at <= now()
      AND state.claim_job_id IS NULL
    ORDER BY state.priority, state.next_run_at, state.created_at, state.track_id
    FOR UPDATE OF state SKIP LOCKED
    LIMIT $4
), claimed AS (
    UPDATE lyrics_lookup_state AS state
    SET claim_job_id = $1,
        claim_job_generation = $2,
        claim_job_lease_id = $3,
        claim_state_generation = state.generation,
        claim_expires_at = now() + $5::bigint * interval '1 second',
        attempts = state.attempts + 1,
        last_attempt_at = now(),
        updated_at = now()
    FROM candidates
    WHERE state.track_id = candidates.track_id
    RETURNING state.track_id,
              state.sc_track_id,
              state.generation,
              state.input_title,
              state.input_artist,
              state.input_duration_ms,
              state.input_genius_song_id,
              state.input_genius_url,
              state.input_release_date,
              state.input_sc_created_at,
              state.miss_streak,
              state.failure_streak
)
SELECT track_id,
       sc_track_id,
       generation,
       input_title,
       input_artist,
       input_duration_ms,
       input_genius_song_id,
       input_genius_url,
       input_release_date,
       input_sc_created_at,
       miss_streak,
       failure_streak
FROM claimed
