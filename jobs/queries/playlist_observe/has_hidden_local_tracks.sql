WITH known AS (
    SELECT sc_track_id
    FROM playlist_track_projection
    WHERE playlist_urn = $1
    UNION
    SELECT track.sc_track_id
    FROM playlist_membership_state AS state
    JOIN playlist_remote_observations AS observation
      ON observation.playlist_urn = state.playlist_urn
     AND observation.id = state.baseline_observation_id
    JOIN playlist_remote_snapshot_tracks AS track
      ON track.snapshot_id = observation.snapshot_id
    WHERE state.playlist_urn = $1
)
SELECT EXISTS(
    SELECT 1
    FROM known
    LEFT JOIN tracks AS track ON track.sc_track_id = known.sc_track_id
    WHERE NOT (known.sc_track_id = ANY ($2))
      AND (track.sc_track_id IS NULL OR track.sharing <> 'public' OR track.deleted_at IS NOT NULL)
) AS "hidden!"
