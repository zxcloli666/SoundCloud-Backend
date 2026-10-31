UPDATE playlists
SET last_read_at = now()
WHERE urn = $1
  AND (last_read_at IS NULL OR last_read_at < now() - INTERVAL '6 hours')
