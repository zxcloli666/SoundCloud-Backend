WITH wanted AS (SELECT DISTINCT id FROM unnest($1::uuid[]) AS u(id))
SELECT w.id AS "artist_id!",
       t.sc_track_id AS "sc_track_id!",
       t.play_count AS "play_count!",
       (t.total > $2) AS "truncated!"
FROM wanted w
         CROSS JOIN LATERAL (SELECT it.sc_track_id,
                                    COALESCE(c.play_count, 0) AS play_count,
                                    ROW_NUMBER() OVER (
                                        ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
                                        ) AS rn,
                                    COUNT(*) OVER () AS total
                             FROM track_artists ta
                                      JOIN tracks it ON it.id = ta.track_id
                                      LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                             WHERE ta.artist_id = w.id
                               AND ta.role = 'primary'
                               AND it.sharing = 'public'
                               AND it.superseded_by IS NULL
                               AND it.storage_state = 'ok'
                               AND it.index_state = 'indexed') t
WHERE t.rn <= $2
ORDER BY w.id, t.rn
