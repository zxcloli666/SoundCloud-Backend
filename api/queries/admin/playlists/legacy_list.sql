SELECT intent.archive_id,
       intent.playlist_urn,
       intent.source,
       intent.classification,
       intent.legacy_user_id,
       intent.legacy_desired_revision,
       intent.legacy_synced_revision,
       intent.queue_last_error,
       intent.archived_at,
       playlist.track_count AS "remote_track_count?",
       state.projection_track_count AS "projection_track_count?",
       state.sync_status AS "sync_status?"
FROM playlist_legacy_membership_intents AS intent
LEFT JOIN playlists AS playlist
  ON playlist.urn = intent.playlist_urn
LEFT JOIN playlist_membership_state AS state
  ON state.playlist_urn = intent.playlist_urn
WHERE intent.resolved_at IS NULL
  AND ($1::text IS NULL OR intent.classification = $1)
ORDER BY intent.archived_at, intent.archive_id
LIMIT $2 OFFSET $3
