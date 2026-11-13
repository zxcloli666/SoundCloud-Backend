WITH page AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.created_at,
           COALESCE(track.index_priority, 5) AS priority,
           cache.source = 'genius'
               AND COALESCE(cache.plain_source, cache.source) = 'genius'
               AND cache.plain_text IS NOT NULL
               AND cache.created_at >= TIMESTAMP '2026-10-28 16:00:00'
               AND cache.created_at < TIMESTAMP '2026-11-13 11:58:01' AS broken
    FROM lyrics_cache AS cache
    LEFT JOIN tracks AS track ON track.sc_track_id = cache.sc_track_id
    WHERE cache.sc_track_id > $1
    ORDER BY cache.sc_track_id
    LIMIT $2
), armed AS (
    INSERT INTO lyrics_repair AS repair (sc_track_id, kind, cache_created_at, priority)
    SELECT page.sc_track_id, 'refetch', page.created_at, page.priority
    FROM page
    WHERE page.broken
    ORDER BY page.sc_track_id
    ON CONFLICT (sc_track_id) DO UPDATE
        SET status = 'pending',
            outcome = NULL,
            attempts = 0,
            next_run_at = now(),
            settled_at = NULL,
            updated_at = now()
        WHERE repair.kind = 'refetch'
          AND repair.outcome IN ('failed', 'abandoned')
    RETURNING 1
)
SELECT (SELECT page.sc_track_id FROM page ORDER BY page.sc_track_id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM page) AS "scanned!",
       (SELECT count(*) FROM armed) AS "armed!"
