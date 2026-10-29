SELECT id
FROM playlist_remote_snapshots
WHERE playlist_urn = $1 AND content_fingerprint = $2
