WITH star AS MATERIALIZED (
    SELECT tiered.sc_track_id, tiered.tiered_at
    FROM star_track_tiers AS tiered
    WHERE tiered.tier = $4
      AND tiered.enrich_due
    ORDER BY tiered.tiered_at, tiered.sc_track_id
    LIMIT $5
), picked AS (
    SELECT track.id
    FROM star
    JOIN tracks AS track
      ON track.sc_track_id = star.sc_track_id
    WHERE track.enrich_state IN ('pending', 'failed')
      AND track.enrich_next_run_at <= now()
      AND (
          track.enrich_locked_at IS NULL
          OR track.enrich_locked_at < now() - $1 * interval '1 second'
      )
      AND track.enrich_attempts < $2
    ORDER BY star.tiered_at, star.sc_track_id
    LIMIT $3
    FOR UPDATE OF track SKIP LOCKED
)
UPDATE tracks AS track
SET enrich_locked_at = now(),
    enrich_attempts = track.enrich_attempts + 1
FROM picked
WHERE track.id = picked.id
RETURNING track.id,
          track.sc_track_id,
          track.enrich_attempts
