WITH soundcloud_plays AS (
    SELECT track.primary_artist_id AS artist_id,
           sum(counter.play_count)::bigint AS plays
    FROM sc_track_counters AS counter
    JOIN tracks AS track ON track.sc_track_id = counter.sc_track_id
    WHERE track.primary_artist_id IS NOT NULL
    GROUP BY track.primary_artist_id
), internal_plays AS (
    SELECT track.primary_artist_id AS artist_id,
           count(*)::bigint AS plays
    FROM user_events AS event
    JOIN tracks AS track ON track.sc_track_id = event.sc_track_id
    WHERE event.event_type = 'full_play'
      AND track.primary_artist_id IS NOT NULL
    GROUP BY track.primary_artist_id
), combined AS (
    SELECT COALESCE(soundcloud_plays.artist_id, internal_plays.artist_id) AS artist_id,
           COALESCE(soundcloud_plays.plays, 0)
               + COALESCE(internal_plays.plays, 0) * $1::bigint AS score
    FROM soundcloud_plays
    FULL JOIN internal_plays USING (artist_id)
), maximum AS (
    SELECT greatest(max(score), 1)::bigint AS score
    FROM combined
), aggregate AS (
    SELECT artist.id,
           COALESCE(
               LEAST(
                   1.0::real,
                   ln(greatest(combined.score, 0) + 1)::real
                       / NULLIF(ln(maximum.score + 1)::real, 0)
               ),
               0
           ) AS popularity_score
    FROM artists AS artist
    CROSS JOIN maximum
    LEFT JOIN combined ON combined.artist_id = artist.id
    WHERE artist.merged_into IS NULL
)
UPDATE artists AS artist
SET popularity_score = aggregate.popularity_score
FROM aggregate
WHERE artist.id = aggregate.id
  AND artist.popularity_score IS DISTINCT FROM aggregate.popularity_score
