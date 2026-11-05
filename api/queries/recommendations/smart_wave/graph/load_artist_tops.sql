WITH wanted AS (SELECT DISTINCT id FROM unnest($1::uuid[]) AS u(id))
SELECT w.id AS "artist_id!",
       t.sc_track_id AS "sc_track_id!",
       t.play_count AS "play_count!",
       (t.indexed AND t.indexed_rn <= $2) AS "wave!",
       (t.indexed_total > $2) AS "wave_truncated!",
       (t.resolved AND t.resolved_rn <= $2) AS "home!",
       (t.resolved_total > $2) AS "home_truncated!"
FROM wanted w
         CROSS JOIN LATERAL (SELECT it.sc_track_id,
                                    COALESCE(c.play_count, 0) AS play_count,
                                    it.index_state = 'indexed' AS indexed,
                                    NOT it.needs_duration_resolve AS resolved,
                                    ROW_NUMBER() OVER (
                                        PARTITION BY it.index_state = 'indexed'
                                        ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
                                        ) AS indexed_rn,
                                    ROW_NUMBER() OVER (
                                        PARTITION BY it.needs_duration_resolve
                                        ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
                                        ) AS resolved_rn,
                                    COUNT(*) FILTER (WHERE it.index_state = 'indexed') OVER () AS indexed_total,
                                    COUNT(*) FILTER (WHERE NOT it.needs_duration_resolve) OVER () AS resolved_total
                             FROM track_artists ta
                                      JOIN tracks it ON it.id = ta.track_id
                                      LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                             WHERE ta.artist_id = w.id
                               AND ta.role = 'primary'
                               AND it.sharing = 'public'
                               AND it.superseded_by IS NULL
                               AND it.storage_state = 'ok'
                               AND (it.index_state = 'indexed' OR NOT it.needs_duration_resolve)) t
WHERE (t.indexed AND t.indexed_rn <= $2)
   OR (t.resolved AND t.resolved_rn <= $2)
ORDER BY w.id, t.play_count DESC, t.sc_track_id
