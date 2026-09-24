WITH picked AS (SELECT id
                FROM artists
                WHERE merged_into IS NULL
                  AND NOT crawl_dead
                  AND mb_artist_id IS NOT NULL
                  AND genius_artist_id IS NULL
                  AND mb_next_run_at <= now()
                  AND (mb_locked_at IS NULL OR mb_locked_at < now() - ($1 * interval '1 second'))
                ORDER BY mb_next_run_at
                LIMIT $2 FOR UPDATE SKIP LOCKED)
UPDATE artists AS artist
SET mb_locked_at = now()
FROM picked
WHERE artist.id = picked.id
RETURNING artist.id,
    artist.mb_artist_id,
    artist.genius_artist_id,
    artist.sc_user_id,
    artist.mb_crawl_offset,
    artist.genius_crawl_offset,
    artist.crawl_fail_count
