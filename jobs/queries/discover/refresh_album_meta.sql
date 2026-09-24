WITH aggregate AS (
    SELECT album.id,
           count(track.id)::integer AS track_count,
           COALESCE(sum(track.duration_ms), 0)::bigint AS total_duration_ms,
           min(track.release_date) AS earliest_release
    FROM albums AS album
    LEFT JOIN album_tracks AS linked ON linked.album_id = album.id
    LEFT JOIN tracks AS track ON track.id = linked.track_id
    GROUP BY album.id
)
UPDATE albums AS album
SET track_count = aggregate.track_count,
    total_duration_ms = aggregate.total_duration_ms,
    release_date = COALESCE(aggregate.earliest_release, album.release_date),
    aggregates_updated_at = now()
FROM aggregate
WHERE album.id = aggregate.id
  AND (album.track_count, album.total_duration_ms, album.release_date)
      IS DISTINCT FROM (
          aggregate.track_count,
          aggregate.total_duration_ms,
          COALESCE(aggregate.earliest_release, album.release_date)
      )
