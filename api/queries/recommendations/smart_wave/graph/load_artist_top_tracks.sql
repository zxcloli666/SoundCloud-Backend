WITH ranked AS (SELECT ta.artist_id,
                       it.sc_track_id,
                       COALESCE(c.play_count, 0) AS play_count,
                       ROW_NUMBER() OVER (
            PARTITION BY ta.artist_id ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
        ) AS rn,
                       COUNT(*) OVER (PARTITION BY ta.artist_id) AS total
                FROM track_artists ta
                         JOIN tracks it ON it.id = ta.track_id
                         LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                WHERE ta.artist_id = ANY ($1)
                  AND ta.role = 'primary'
                  AND it.sharing = 'public'
                  AND it.superseded_by IS NULL
                  AND it.storage_state = 'ok'
                  AND it.index_state = 'indexed')
SELECT artist_id AS "artist_id!",
       sc_track_id AS "sc_track_id!",
       play_count AS "play_count!",
       (total > $2) AS "truncated!"
FROM ranked
WHERE rn <= $2
ORDER BY artist_id, rn
