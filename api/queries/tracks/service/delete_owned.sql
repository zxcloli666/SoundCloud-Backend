DELETE FROM user_owned_tracks
WHERE user_id = ANY($1) AND sc_track_id = $2
