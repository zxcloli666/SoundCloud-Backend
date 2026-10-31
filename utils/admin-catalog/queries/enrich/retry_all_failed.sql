WITH locked AS MATERIALIZED (SELECT track.id
                             FROM tracks track
                             WHERE track.enrich_state IN ('failed', 'dead')
                             ORDER BY track.sc_track_id
                                 FOR NO KEY UPDATE OF track)
UPDATE tracks
SET enrich_state       = 'pending',
    enrich_attempts    = 0,
    enriched_at        = NULL,
    enrich_source      = NULL,
    enrich_confidence  = NULL,
    enrich_next_run_at = now(),
    enrich_locked_at   = NULL,
    enrich_error       = NULL
FROM locked
WHERE tracks.id = locked.id
