SELECT track.sc_track_id,
       cdn.updated_at AS uploaded_at
FROM cdn_tracks AS cdn
JOIN tracks AS track ON track.urn = cdn.track_urn
LEFT JOIN storage_event_state AS event ON event.sc_track_id = track.sc_track_id
LEFT JOIN storage_event_repairs AS repair ON repair.sc_track_id = track.sc_track_id
WHERE cdn.status = 'ok'
  AND cdn.quality = 'single'
  AND cdn.updated_at > LOCALTIMESTAMP - $2::bigint * interval '1 second'
  AND cdn.updated_at < LOCALTIMESTAMP - $3::bigint * interval '1 second'
  AND track.storage_state NOT IN ('ok', 'too_long')
  AND (event.updated_at IS NULL OR event.updated_at < cdn.updated_at - interval '1 minute')
  AND (repair.uploaded_at IS NULL OR repair.uploaded_at < cdn.updated_at)
ORDER BY cdn.updated_at
LIMIT $1
