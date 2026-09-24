WITH picked AS (SELECT id
                FROM artists
                WHERE merged_into IS NULL
                  AND NOT crawl_dead
                  AND has_sc_account
                  AND genius_artist_id IS NULL
                  AND genius_next_run_at <= now()
                  AND (genius_locked_at IS NULL OR genius_locked_at < now() - ($1 * interval '1 second'))
                ORDER BY genius_next_run_at
                LIMIT $2 FOR UPDATE SKIP LOCKED)
UPDATE artists AS artist
SET genius_locked_at = now()
FROM picked
WHERE artist.id = picked.id
RETURNING artist.id, artist.name
