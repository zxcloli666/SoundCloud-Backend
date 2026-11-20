WITH page AS MATERIALIZED (
    SELECT cache.sc_track_id
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id > $1
    ORDER BY cache.sc_track_id
    LIMIT $2
), moved AS (
    UPDATE lyrics_cache AS cache
    SET fts = cache.fts || ''::tsvector
    FROM page
    WHERE cache.sc_track_id = page.sc_track_id
      AND pg_column_toast_chunk_id(cache.fts) IS NOT NULL
      AND pg_column_size(cache.fts) < $3
    RETURNING cache.sc_track_id
)
SELECT (SELECT page.sc_track_id FROM page ORDER BY page.sc_track_id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM page) AS "scanned!",
       (SELECT count(*) FROM moved) AS "moved!"
