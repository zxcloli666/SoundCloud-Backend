UPDATE wanted_tracks
SET track_id   = target.id,
    status     = 'linked',
    updated_at = now()
FROM (SELECT COALESCE(winner.id, track.id) AS id
      FROM tracks AS track
               LEFT JOIN tracks AS winner ON winner.id = track.superseded_by
      WHERE track.sc_track_id = $2
      LIMIT 1) AS target
WHERE wanted_tracks.id = $1
RETURNING track_id
