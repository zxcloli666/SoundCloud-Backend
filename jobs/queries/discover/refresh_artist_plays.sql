WITH activity AS (
    SELECT track.primary_artist_id AS artist_id,
           count(DISTINCT event.sc_user_id) FILTER (
               WHERE event.event_type IN ('full_play', 'like', 'playlist_add')
           )::bigint AS listeners_30d,
           count(*) FILTER (
               WHERE event.event_type = 'full_play'
                 AND event.created_at > now() - interval '7 days'
           )::bigint AS plays_7d,
           count(*) FILTER (
               WHERE event.event_type = 'full_play'
           )::bigint AS plays_30d
    FROM user_events AS event
    JOIN tracks AS track ON track.sc_track_id = event.sc_track_id
    WHERE event.created_at > now() - interval '30 days'
      AND track.primary_artist_id IS NOT NULL
      AND (
          $1::bigint = 1
          OR ((hashtextextended(track.primary_artist_id::text, 0) % $1::bigint) + $1::bigint)
              % $1::bigint = $2::bigint
      )
    GROUP BY track.primary_artist_id
), aggregate AS (
    SELECT artist.id,
           COALESCE(activity.listeners_30d, 0) AS listeners_30d,
           LEAST(
               1.0::real,
               GREATEST(
                   0.0::real,
                   (
                       COALESCE(activity.plays_7d, 0)::real * 30.0
                       / (7.0 * (COALESCE(activity.plays_30d, 0)::real + 1.0))
                       - 0.5
                   ) / 3.0
               )
           ) AS trending_score
    FROM artists AS artist
    LEFT JOIN activity ON activity.artist_id = artist.id
    WHERE artist.merged_into IS NULL
      AND (
          $1::bigint = 1
          OR ((hashtextextended(artist.id::text, 0) % $1::bigint) + $1::bigint)
              % $1::bigint = $2::bigint
      )
)
UPDATE artists AS artist
SET monthly_listeners = aggregate.listeners_30d,
    trending_score = aggregate.trending_score
FROM aggregate
WHERE artist.id = aggregate.id
  AND (artist.monthly_listeners, artist.trending_score)
      IS DISTINCT FROM (aggregate.listeners_30d, aggregate.trending_score)
