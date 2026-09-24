WITH plays AS (
    SELECT linked.album_id,
           sum(COALESCE(counter.play_count, 0))::bigint AS play_count
    FROM album_tracks AS linked
    JOIN tracks AS track ON track.id = linked.track_id
    LEFT JOIN sc_track_counters AS counter ON counter.sc_track_id = track.sc_track_id
    GROUP BY linked.album_id
), maximum AS (
    SELECT greatest(max(play_count), 1)::bigint AS play_count
    FROM plays
), aggregate AS (
    SELECT album.id,
           COALESCE(
               LEAST(
                   1.0::real,
                   ln(greatest(plays.play_count, 0) + 1)::real
                       / NULLIF(ln(maximum.play_count + 1)::real, 0)
               ),
               0
           ) AS popularity_score
    FROM albums AS album
    CROSS JOIN maximum
    LEFT JOIN plays ON plays.album_id = album.id
)
UPDATE albums AS album
SET popularity_score = aggregate.popularity_score
FROM aggregate
WHERE album.id = aggregate.id
  AND album.popularity_score IS DISTINCT FROM aggregate.popularity_score
