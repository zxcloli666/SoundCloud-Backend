INSERT INTO playlist_remote_snapshots (
    playlist_urn,
    content_fingerprint,
    track_count
)
VALUES ($1, $2, $3)
ON CONFLICT (playlist_urn, content_fingerprint) DO UPDATE
SET track_count = playlist_remote_snapshots.track_count
RETURNING id
