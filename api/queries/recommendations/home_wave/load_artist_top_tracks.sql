WITH wanted AS (SELECT DISTINCT id FROM unnest($1::uuid[]) AS u(id))
SELECT w.id AS "artist_id!",
       t.sc_track_id AS "sc_track_id!",
       t.play_count AS "play_count!",
       (t.total > $2) AS "truncated!"
FROM wanted w
         CROSS JOIN LATERAL (SELECT p.sc_track_id,
                                    COALESCE(c.play_count, 0) AS play_count,
                                    ROW_NUMBER() OVER (
                                        ORDER BY COALESCE(c.play_count, 0) DESC, p.sc_track_id
                                        ) AS rn,
                                    COUNT(*) OVER () AS total
                             FROM artist_playable_tracks p
                                      LEFT JOIN sc_track_counters c ON c.sc_track_id = p.sc_track_id
                             WHERE p.artist_id = w.id
                               AND p.ready_home) t
WHERE t.rn <= $2
ORDER BY w.id, t.rn
