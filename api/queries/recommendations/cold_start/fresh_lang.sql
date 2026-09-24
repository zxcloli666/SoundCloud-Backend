SELECT sc_track_id
FROM tracks
WHERE sc_synced_at > NOW() - make_interval(days => $2::int)
  AND language = ANY ($1)
  AND sharing = 'public'
  AND superseded_by IS NULL
ORDER BY sc_synced_at DESC
    LIMIT $3
