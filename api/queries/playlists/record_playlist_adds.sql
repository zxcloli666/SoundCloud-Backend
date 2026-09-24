INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, weight)
SELECT added.id, $1, added.sc_track_id, 'playlist_add', $4
FROM unnest($2::uuid[], $3::text[]) AS added(id, sc_track_id)
WHERE NOT EXISTS (
    SELECT 1
    FROM disliked_tracks AS disliked
    WHERE disliked.sc_user_id = ANY ($5::text[])
      AND disliked.sc_track_id = added.sc_track_id
)
ON CONFLICT (id) DO NOTHING
