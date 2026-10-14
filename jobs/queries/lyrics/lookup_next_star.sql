WITH star AS MATERIALIZED (
    SELECT tiered.sc_track_id, tiered.tiered_at
    FROM star_track_tiers AS tiered
    WHERE tiered.tier = $1
      AND tiered.lookup_due
    ORDER BY tiered.tiered_at, tiered.sc_track_id
    LIMIT $2
)
SELECT greatest(state.next_run_at, star.tiered_at) AS "waiting_since!"
FROM star
JOIN lyrics_lookup_state AS state
  ON state.sc_track_id = star.sc_track_id
WHERE state.next_run_at <= now()
  AND state.claim_job_id IS NULL
ORDER BY star.tiered_at, star.sc_track_id
LIMIT 1
