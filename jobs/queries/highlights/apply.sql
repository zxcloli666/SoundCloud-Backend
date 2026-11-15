WITH track AS (
    SELECT id
    FROM tracks
    WHERE sc_track_id = $1
), stored AS (
    INSERT INTO track_highlights AS highlight (track_id, start_ms, length_ms, score, version, computed_at)
    SELECT track.id, $3, $4, $5, $6, now()
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
    UPDATE track_highlight_dispatch AS dispatch
    SET state = 'done',
        reason = NULL,
        updated_at = now()
    FROM track
    WHERE dispatch.track_id = track.id
      AND dispatch.attempt = $2
      AND dispatch.state = 'dispatched'
    RETURNING dispatch.track_id
)
SELECT (SELECT count(*) FROM stored) AS "stored!",
       (SELECT count(*) FROM settled) AS "settled!"
