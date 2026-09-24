SELECT track.id AS track_id,
       cache.synced_lrc AS "synced_lrc?",
       cache.plain_text AS "plain_text?",
       cache.source AS "source?",
       cache.language AS "language?",
       cache.language_confidence AS "language_confidence?",
       state.status AS "lookup_status?",
       state.next_run_at AS "next_run_at?",
       state.failure_streak AS "failure_streak?"
FROM tracks AS track
LEFT JOIN lyrics_cache AS cache
  ON cache.sc_track_id = track.sc_track_id
 AND (
     NULLIF(btrim(cache.plain_text), '') IS NOT NULL
     OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL
 )
LEFT JOIN lyrics_lookup_state AS state ON state.track_id = track.id
WHERE track.sc_track_id = $1
