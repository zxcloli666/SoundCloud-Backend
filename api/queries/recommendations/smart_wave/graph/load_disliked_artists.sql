WITH disl AS (SELECT ta.artist_id, COUNT(*) AS dc
              FROM disliked_tracks dt
                       JOIN tracks it ON it.sc_track_id = dt.sc_track_id
                       JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
              WHERE dt.sc_user_id = ANY ($1)
              GROUP BY ta.artist_id),
     lik AS (SELECT ta.artist_id, COUNT(*) AS lc
             FROM user_likes_tracks ul
                      JOIN tracks it ON it.sc_track_id = ul.sc_track_id
                      JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
             WHERE ul.user_id = ANY ($1)
               AND ul.wanted_state = true
             GROUP BY ta.artist_id),
     blocked AS (SELECT kind, target_id
                 FROM user_blocked_artists
                 WHERE sc_user_id = ANY ($1))
SELECT d.artist_id AS "artist_id!"
FROM disl d
         LEFT JOIN lik l ON l.artist_id = d.artist_id
WHERE d.dc >= $2
  AND d.dc > COALESCE(l.lc, 0)
UNION
SELECT a.id AS "artist_id!"
FROM blocked b
         JOIN artists a ON a.id = CASE WHEN b.kind = 'artist' THEN b.target_id::uuid END
UNION
SELECT asa.artist_id AS "artist_id!"
FROM blocked b
         JOIN artist_sc_accounts asa ON asa.sc_user_id = b.target_id AND asa.role = 'main'
WHERE b.kind = 'user'
