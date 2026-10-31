SELECT track.id,
       track.sc_track_id,
       track.canonical_track_id,
       substr(track.audio_fingerprint, 1, 64) AS prefix
FROM tracks AS track
WHERE track.sc_track_id = $1
   OR track.id = $2
ORDER BY track.sc_track_id
FOR NO KEY UPDATE
