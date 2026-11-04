WITH wanted AS (SELECT DISTINCT id FROM unnest($1::uuid[]) AS u(id))
SELECT w.id AS "artist_id!", t.sc_track_id AS "sc_track_id!"
FROM wanted w
         CROSS JOIN LATERAL (SELECT it.sc_track_id,
                                    ROW_NUMBER() OVER (ORDER BY COALESCE(c.play_count, 0) DESC) AS rn
                             FROM track_artists ta
                                      JOIN tracks it ON it.id = ta.track_id
                                      LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                             WHERE ta.artist_id = w.id
                               AND ta.role = 'primary'
                               AND it.sharing = 'public'
                               AND it.superseded_by IS NULL
                               AND it.storage_state = 'ok'
                               AND it.index_state = 'indexed'
                               AND NOT (it.sc_track_id = ANY ($2))) t
WHERE t.rn <= $3
ORDER BY w.id, t.rn
LIMIT $4
