WITH star AS MATERIALIZED (
    SELECT tiered.sc_track_id, tiered.tiered_at
    FROM star_track_tiers AS tiered
    WHERE tiered.tier = $3
      AND tiered.enrich_due
    ORDER BY tiered.tiered_at, tiered.sc_track_id
    LIMIT $4
)
SELECT greatest(queue.next_run_at, star.tiered_at) AS "waiting_since!"
FROM star
JOIN tracks AS track
  ON track.sc_track_id = star.sc_track_id
JOIN track_enrich_queue AS queue
  ON queue.track_id = track.id
WHERE queue.enrich_state IN ('pending', 'failed')
  AND queue.next_run_at <= now()
  AND (
      queue.locked_at IS NULL
      OR queue.locked_at < now() - $1 * interval '1 second'
  )
  AND queue.attempts < $2
ORDER BY star.tiered_at, star.sc_track_id
LIMIT 1
