WITH walked AS (
    SELECT track.id,
           track.deleted_at IS NULL AS live,
           COALESCE(track.play_count_sc, 0) + 10 * COALESCE(track.likes_count_sc, 0) AS popularity
    FROM tracks AS track
    WHERE track.sharing = 'public'
      AND track.superseded_by IS NULL
      AND track.storage_state = 'ok'
      AND track.index_state = 'indexed'
      AND track.id > $1
    ORDER BY track.id
    LIMIT $2
), queued AS (
    INSERT INTO track_highlight_dispatch AS dispatch (track_id, version, popularity)
    SELECT walked.id, $3, walked.popularity
    FROM walked
    WHERE walked.live
      AND NOT EXISTS (
          SELECT 1
          FROM track_highlights AS highlight
          WHERE highlight.track_id = walked.id
            AND highlight.version >= $3
      )
    ON CONFLICT (track_id) DO UPDATE
    SET version = EXCLUDED.version,
        state = 'queued',
        popularity = EXCLUDED.popularity,
        failures = 0,
        next_run_at = now(),
        reason = NULL,
        updated_at = now()
    WHERE dispatch.version < EXCLUDED.version
      AND dispatch.state <> 'dispatched'
    RETURNING dispatch.track_id
)
SELECT (SELECT walked.id FROM walked ORDER BY walked.id DESC LIMIT 1) AS last,
       (SELECT count(*) FROM walked) AS "walked!",
       (SELECT count(*) FROM queued) AS "queued!"
