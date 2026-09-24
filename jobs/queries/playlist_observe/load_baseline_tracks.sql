SELECT track.sc_track_id
FROM playlist_membership_state AS state
JOIN playlist_remote_observations AS observation
  ON observation.playlist_urn = state.playlist_urn
 AND observation.id = state.baseline_observation_id
JOIN playlist_remote_snapshot_tracks AS track
  ON track.snapshot_id = observation.snapshot_id
WHERE state.playlist_urn = $1
ORDER BY track.position
