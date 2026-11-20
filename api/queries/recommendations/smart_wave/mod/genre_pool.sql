SELECT t.sc_track_id AS "sc_track_id!",
       t.primary_artist_id
FROM unnest($1::text[]) AS g(name)
         CROSS JOIN LATERAL (SELECT sc_track_id, primary_artist_id
                             FROM tracks
                             WHERE lower(genre) = g.name
                               AND sharing = 'public'
                               AND deleted_at IS NULL
                               AND superseded_by IS NULL
                               AND storage_state = 'ok'
                               AND index_state = 'indexed'
                               AND NOT (sc_track_id = ANY ($2))
                             ORDER BY play_count_sc DESC NULLS LAST
                             LIMIT $3) t
