WITH owned_job AS MATERIALIZED (
    SELECT id
    FROM background_jobs
    WHERE id = $1
      AND kind = 'lyrics.lookup'
      AND generation = $2
      AND lease_generation = $2
      AND lease_id = $3
      AND lease_expires_at > now()
), candidate AS MATERIALIZED (
    SELECT queue.track_id
    FROM lyrics_lookup_state AS state
    JOIN lyrics_lookup_queue AS queue
      ON queue.track_id = state.track_id
    JOIN owned_job ON true
    WHERE state.sc_track_id = $4
      AND queue.next_run_at <= now()
      AND queue.claim_job_id IS NULL
    FOR UPDATE OF queue SKIP LOCKED
), claimed AS (
    UPDATE lyrics_lookup_queue AS queue
    SET claim_job_id = $1,
        claim_job_generation = $2,
        claim_job_lease_id = $3,
        claim_state_generation = state.generation,
        claim_expires_at = now() + $5::bigint * interval '1 second',
        attempts = queue.attempts + 1,
        last_attempt_at = now()
    FROM candidate
    JOIN lyrics_lookup_state AS state ON state.track_id = candidate.track_id
    WHERE queue.track_id = candidate.track_id
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
