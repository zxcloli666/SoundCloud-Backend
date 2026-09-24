WITH ul AS (
    SELECT user_id,
           sc_track_id,
           row_number() OVER (
               PARTITION BY user_id
               ORDER BY created_at DESC, sc_track_id DESC
           ) AS rn
    FROM user_likes_tracks
    WHERE wanted_state = true
), ua AS (
    SELECT DISTINCT ul.user_id, track_artist.artist_id
    FROM ul
    JOIN tracks AS track ON track.sc_track_id = ul.sc_track_id
    JOIN track_artists AS track_artist
        ON track_artist.track_id = track.id
       AND track_artist.role = 'primary'
    JOIN artists AS artist
        ON artist.id = track_artist.artist_id
       AND artist.merged_into IS NULL
    WHERE ul.rn <= 500
), counts AS (
    SELECT artist_id, count(*) AS likers
    FROM ua
    GROUP BY artist_id
), pairs AS (
    SELECT left_artist.artist_id AS a,
           right_artist.artist_id AS b,
           count(*) AS co
    FROM ua AS left_artist
    JOIN ua AS right_artist
        ON right_artist.user_id = left_artist.user_id
       AND left_artist.artist_id < right_artist.artist_id
    GROUP BY left_artist.artist_id, right_artist.artist_id
    HAVING count(*) >= 2
), weighted AS (
    SELECT pair.a,
           pair.b,
           pair.co,
           (
               pair.co
               / (sqrt(left_count.likers::float8 * right_count.likers::float8) + $1)
           )::real AS weight
    FROM pairs AS pair
    JOIN counts AS left_count ON left_count.artist_id = pair.a
    JOIN counts AS right_count ON right_count.artist_id = pair.b
), ranked AS (
    SELECT a,
           b,
           co,
           weight,
           row_number() OVER (PARTITION BY a ORDER BY weight DESC) AS rank_a,
           row_number() OVER (PARTITION BY b ORDER BY weight DESC) AS rank_b
    FROM weighted
)
INSERT INTO artist_colike (a_id, b_id, co, w, updated_at)
SELECT a, b, co::int, weight, now()
FROM ranked
WHERE rank_a <= $2 OR rank_b <= $2
ON CONFLICT (a_id, b_id) DO UPDATE
SET co = excluded.co,
    w = excluded.w,
    updated_at = excluded.updated_at
