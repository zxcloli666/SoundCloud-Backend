WITH recent AS (SELECT l.sc_track_id, l.created_at
                FROM unnest($1::text[]) AS u(id)
                         CROSS JOIN LATERAL (SELECT sc_track_id, created_at
                                             FROM user_likes_tracks
                                             WHERE user_id = u.id
                                               AND wanted_state = true
                                               AND created_at > now() - make_interval(days => $2::int)
                                             ORDER BY created_at DESC, sc_track_id DESC
                                             LIMIT 200) l
                ORDER BY l.created_at DESC, l.sc_track_id DESC
                LIMIT 200
    )
   , superliked AS (SELECT s.sc_track_id, GREATEST(s.created_at, l.created_at) AS created_at
                    FROM user_superlikes s
                    JOIN user_likes_tracks l
                      ON l.user_id = s.user_id
                     AND l.sc_track_id = s.sc_track_id
                     AND l.created_at = s.like_created_at
                    WHERE s.user_id = ANY ($1)
                      AND l.wanted_state = true
                    ORDER BY s.created_at DESC
    LIMIT 100
    )
   , rl AS (SELECT COALESCE(su.sc_track_id, re.sc_track_id) AS sc_track_id,
                   (EXP(-EXTRACT(EPOCH FROM (now() - COALESCE(su.created_at, re.created_at))) / 86400.0 / 60.0)
                       * CASE WHEN su.sc_track_id IS NULL THEN 1.0 ELSE $6::real END) ::real AS rec
            FROM recent re
            FULL OUTER JOIN superliked su ON su.sc_track_id = re.sc_track_id
    )
   , parts AS (
SELECT ta.artist_id, rl.rec, (CASE ta.role WHEN 'primary' THEN 1.0 WHEN 'featured' THEN 0.6 ELSE 0.5 END):: real AS rw
FROM rl
    JOIN tracks it
ON it.sc_track_id = rl.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role IN ('primary','featured','remixer')
UNION ALL
SELECT aa.artist_id, rl.rec, 0.8:: real
FROM rl
    JOIN tracks it
ON it.sc_track_id = rl.sc_track_id
    JOIN album_artists aa ON aa.album_id = it.album_id
WHERE NOT EXISTS (
    SELECT 1 FROM track_artists ta2
    WHERE ta2.track_id = it.id
  AND ta2.role IN ('primary'
    , 'featured'
    , 'remixer')
    )
    )
    , like_w AS (
SELECT artist_id, SUM (rec*rw):: real AS w
FROM parts
GROUP BY artist_id),
    play_w AS (
SELECT ta.artist_id, SUM (EXP(- EXTRACT (EPOCH FROM (now()-ue.created_at))/86400.0/60.0)):: real AS w
FROM user_events ue
    JOIN tracks it
ON it.sc_track_id = ue.sc_track_id
    JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
WHERE ue.sc_user_id = ANY ($1)
  AND ue.event_type IN ('full_play'
    , 'play_complete')
  AND ue.created_at
    > now() - make_interval(days => $3:: int)
GROUP BY ta.artist_id
    ),
    merged AS (
SELECT COALESCE (l.artist_id, p.artist_id) AS artist_id, (COALESCE (l.w, 0) + $4:: real * COALESCE (p.w, 0)):: real AS weight
FROM like_w l FULL OUTER JOIN play_w p
ON p.artist_id = l.artist_id
    )
SELECT m.artist_id AS "artist_id!", m.weight AS "weight!"
FROM merged m
         JOIN artists a ON a.id = m.artist_id
WHERE a.merged_into IS NULL
  AND m.weight > 0
ORDER BY m.weight DESC LIMIT $5
