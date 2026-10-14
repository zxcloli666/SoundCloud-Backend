WITH star AS MATERIALIZED (
    SELECT tiered.sc_track_id, tiered.tiered_at
    FROM star_track_tiers AS tiered
    WHERE tiered.tier = $3
      AND tiered.enrich_due
    ORDER BY tiered.tiered_at, tiered.sc_track_id
    LIMIT $4
)
SELECT greatest(track.enrich_next_run_at, star.tiered_at) AS "waiting_since!"
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
LIMIT 1
