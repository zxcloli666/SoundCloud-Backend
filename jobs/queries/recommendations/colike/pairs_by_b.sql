WITH pairs AS (
    SELECT left_artist.artist_id AS a,
           right_artist.artist_id AS b,
           count(*) AS co
    FROM colike_users AS left_artist
    JOIN colike_users AS right_artist
        ON right_artist.user_id = left_artist.user_id
       AND left_artist.artist_id < right_artist.artist_id
    WHERE right_artist.shard = $3
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
    JOIN colike_likers AS left_count ON left_count.artist_id = pair.a
    JOIN colike_likers AS right_count ON right_count.artist_id = pair.b
), ranked AS (
    SELECT a,
           b,
           co,
           weight,
           row_number() OVER (PARTITION BY b ORDER BY weight DESC, a) AS rank
    FROM weighted
)
INSERT INTO colike_fresh (a, b, co, weight)
SELECT a, b, co::int, weight
FROM ranked
WHERE rank <= $2
ON CONFLICT (a, b) DO NOTHING
