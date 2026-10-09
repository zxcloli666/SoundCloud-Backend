INSERT INTO user_owned_tracks (user_id, sc_track_id, progress, synced_at, created_at)
SELECT $1, $2, false, now(), now()
WHERE NOT EXISTS (
    SELECT 1 FROM user_owned_tracks existing
    WHERE existing.user_id = ANY($3) AND existing.sc_track_id = $2
)
ON CONFLICT (user_id, sc_track_id) DO NOTHING
