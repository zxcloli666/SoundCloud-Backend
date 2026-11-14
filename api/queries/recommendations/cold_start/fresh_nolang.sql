SELECT track.sc_track_id
FROM track_sync_stamps AS stamp
JOIN tracks AS track ON track.id = stamp.track_id
WHERE stamp.sc_synced_at > NOW() - make_interval(days => $1::int)
  AND track.sharing = 'public'
  AND track.superseded_by IS NULL
ORDER BY stamp.sc_synced_at DESC LIMIT $2
