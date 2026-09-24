WITH active_users AS MATERIALIZED (
    SELECT DISTINCT regexp_replace(sc_user_id, '^soundcloud:users:', '') AS user_id
    FROM user_events
    WHERE created_at > now() - interval '14 days'
      AND (
          $1::bigint = 1
          OR ((hashtextextended(regexp_replace(sc_user_id, '^soundcloud:users:', ''), 0) % $1::bigint)
              + $1::bigint) % $1::bigint = $2::bigint
      )
), seed AS (
    SELECT active.user_id,
           track_artist.artist_id,
           row_number() OVER (
               PARTITION BY active.user_id
               ORDER BY count(*) DESC, track_artist.artist_id
           ) AS rank
    FROM active_users AS active
    JOIN user_likes_tracks AS user_like
        ON user_like.user_id = active.user_id
       AND user_like.wanted_state = true
    JOIN tracks AS track ON track.sc_track_id = user_like.sc_track_id
    JOIN track_artists AS track_artist
        ON track_artist.track_id = track.id
       AND track_artist.role = 'primary'
    WHERE user_like.created_at > now() - interval '180 days'
    GROUP BY active.user_id, track_artist.artist_id
), neighborhood AS (
    SELECT artist_id
    FROM seed
    WHERE rank <= 12
    UNION
    SELECT CASE
               WHEN edge.a_id = seed_artist.artist_id THEN edge.b_id
               ELSE edge.a_id
           END
    FROM (
        SELECT DISTINCT artist_id
        FROM seed
        WHERE rank <= 6
    ) AS seed_artist
    JOIN LATERAL (
        SELECT a_id, b_id
        FROM artist_colike
        WHERE a_id = seed_artist.artist_id OR b_id = seed_artist.artist_id
        ORDER BY w DESC
        LIMIT 8
    ) AS edge ON true
), candidates AS (
    SELECT candidate.id,
           row_number() OVER (
               PARTITION BY track_artist.artist_id
               ORDER BY coalesce(counter.play_count, 0) DESC
           ) AS rank
    FROM neighborhood
    JOIN track_artists AS track_artist
        ON track_artist.artist_id = neighborhood.artist_id
       AND track_artist.role = 'primary'
    JOIN tracks AS candidate ON candidate.id = track_artist.track_id
    LEFT JOIN sc_track_counters AS counter
        ON counter.sc_track_id = candidate.sc_track_id
    WHERE candidate.sharing = 'public'
      AND candidate.storage_state = 'pending'
      AND (candidate.storage_priority > 0 OR candidate.index_priority > 0)
)
UPDATE tracks
SET storage_priority = 0,
    index_priority = 0
WHERE id IN (
    SELECT id
    FROM candidates
    WHERE rank <= 4
)
