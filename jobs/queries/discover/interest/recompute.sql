WITH activity AS MATERIALIZED (
    SELECT track_artist.artist_id,
           count(*)::real AS score
    FROM user_events AS event
    JOIN tracks AS track ON track.sc_track_id = event.sc_track_id
    JOIN track_artists AS track_artist ON track_artist.track_id = track.id
    WHERE event.created_at > now() - interval '30 days'
    GROUP BY track_artist.artist_id
), scored AS (
    INSERT INTO artist_interest AS interest (artist_id, score)
    SELECT artist.id, activity.score
    FROM activity
    JOIN artists AS artist ON artist.id = activity.artist_id
    LEFT JOIN artist_interest AS stored ON stored.artist_id = artist.id
    WHERE stored.score IS DISTINCT FROM activity.score
      AND (
          $1::bigint = 1
          OR ((hashtextextended(artist.id::text, 0) % $1::bigint) + $1::bigint) % $1::bigint
              = $2::bigint
      )
    ON CONFLICT (artist_id) DO UPDATE
    SET score = EXCLUDED.score
    RETURNING interest.artist_id
), expired AS (
    DELETE FROM artist_interest AS interest
    WHERE (
          $1::bigint = 1
          OR ((hashtextextended(interest.artist_id::text, 0) % $1::bigint) + $1::bigint) % $1::bigint
              = $2::bigint
      )
      AND NOT EXISTS (
          SELECT 1
          FROM activity
          WHERE activity.artist_id = interest.artist_id
      )
    RETURNING interest.artist_id
)
SELECT (SELECT count(*) FROM scored) AS "scored!",
       (SELECT count(*) FROM expired) AS "expired!"
