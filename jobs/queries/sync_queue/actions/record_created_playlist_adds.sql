INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at)
SELECT added.id,
       $2,
       added.sc_track_id,
       'playlist_add',
       $5,
       queued.created_at AT TIME ZONE 'UTC'
FROM sync_queue AS queued
CROSS JOIN unnest($3::uuid[], $4::text[]) AS added(id, sc_track_id)
WHERE queued.id = $1
  AND added.sc_track_id ~ '^[0-9]{1,18}$'
  AND NOT EXISTS (
      SELECT 1
      FROM disliked_tracks AS disliked
      WHERE disliked.sc_user_id = ANY ($6::text[])
        AND disliked.sc_track_id = added.sc_track_id
  )
ON CONFLICT (id) DO NOTHING
