WITH track AS (
    SELECT id,
           COALESCE(play_count_sc, 0) + 10 * COALESCE(likes_count_sc, 0) AS popularity
    FROM tracks
    WHERE sc_track_id = $1
), stored AS (
    INSERT INTO track_highlights AS highlight (track_id, start_ms, length_ms, score, version, computed_at)
    SELECT track.id, $2, $3, $4, $5, now()
    FROM track
    ON CONFLICT (track_id) DO UPDATE
    SET start_ms = EXCLUDED.start_ms,
        length_ms = EXCLUDED.length_ms,
        score = EXCLUDED.score,
        version = EXCLUDED.version,
        computed_at = EXCLUDED.computed_at
    WHERE highlight.version <= EXCLUDED.version
    RETURNING highlight.track_id
), settled AS (
    INSERT INTO track_highlight_dispatch AS dispatch (track_id, version, popularity, state)
    SELECT track.id, $5, track.popularity, 'done'
    FROM track
    ON CONFLICT (track_id) DO UPDATE
    SET version = EXCLUDED.version,
        state = 'done',
        failures = 0,
        reason = NULL,
        updated_at = now()
    WHERE dispatch.state <> 'dispatched'
      AND dispatch.version <= EXCLUDED.version
    RETURNING dispatch.track_id
)
SELECT (SELECT count(*) FROM stored) AS "stored!",
       (SELECT count(*) FROM settled) AS "settled!"
