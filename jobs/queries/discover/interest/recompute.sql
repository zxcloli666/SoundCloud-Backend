WITH activity AS MATERIALIZED (
    SELECT track_artist.artist_id,
           count(*)::real AS score
    FROM user_events AS event
    JOIN tracks AS track ON track.sc_track_id = event.sc_track_id
    JOIN track_artists AS track_artist ON track_artist.track_id = track.id
    WHERE event.created_at > now() - interval '30 days'
    GROUP BY track_artist.artist_id
), scored AS (
    UPDATE artists AS artist
    SET interest_score = activity.score
    FROM activity
    WHERE artist.id = activity.artist_id
      AND artist.interest_score IS DISTINCT FROM activity.score
      AND (
          $1::bigint = 1
          OR ((hashtextextended(artist.id::text, 0) % $1::bigint) + $1::bigint) % $1::bigint
              = $2::bigint
      )
    RETURNING artist.id
), expired AS (
    UPDATE artists AS artist
    SET interest_score = 0
    WHERE artist.interest_score > 0
      AND (
          $1::bigint = 1
          OR ((hashtextextended(artist.id::text, 0) % $1::bigint) + $1::bigint) % $1::bigint
              = $2::bigint
      )
      AND NOT EXISTS (
          SELECT 1
          FROM activity
          WHERE activity.artist_id = artist.id
      )
    RETURNING artist.id
)
SELECT (SELECT count(*) FROM scored) AS "scored!",
       (SELECT count(*) FROM expired) AS "expired!"
