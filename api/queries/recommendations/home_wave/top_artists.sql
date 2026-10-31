WITH liked AS (SELECT sc_track_id
               FROM user_likes_tracks
               WHERE user_id = ANY ($1)
                 AND wanted_state = true
                 AND created_at > NOW() - INTERVAL '180 days'
ORDER BY created_at DESC, ctid DESC
    LIMIT 300
    ),
    like_art AS (
SELECT ta.artist_id, COUNT (*):: real AS lc
FROM liked l
    JOIN tracks it
ON it.sc_track_id = l.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
GROUP BY ta.artist_id
    ),
    play_art AS (
SELECT ta.artist_id, COUNT (DISTINCT ue.sc_track_id):: real AS pc
FROM user_events ue
    JOIN tracks it
ON it.sc_track_id = ue.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
WHERE ue.sc_user_id = ANY ($1)
  AND ue.event_type IN ('full_play'
    , 'play_complete')
  AND ue.created_at
    > NOW() - INTERVAL '180 days'
GROUP BY ta.artist_id
    ),
    top_artists AS (
SELECT COALESCE (la.artist_id, pa.artist_id) AS artist_id, (COALESCE (la.lc, 0) + 0.5 * COALESCE (pa.pc, 0)) AS score
FROM like_art la FULL OUTER JOIN play_art pa
ON pa.artist_id = la.artist_id
ORDER BY score DESC
    LIMIT $2
    )
SELECT a.id AS "artist_id!", a.name AS "artist_name!", a.avatar_url
FROM top_artists tt
         JOIN artists a ON a.id = tt.artist_id
WHERE a.merged_into IS NULL
ORDER BY tt.score DESC, a.id
