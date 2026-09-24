SELECT state.sc_track_id
FROM lyrics_lookup_state AS state
JOIN tracks AS track ON track.id = state.track_id
WHERE state.wake_message_id IS NOT NULL
  AND state.wake_durable_at IS NULL
  AND (
      track.primary_artist_id = $1
      OR EXISTS (
          SELECT 1
          FROM track_artists AS credit
          WHERE credit.track_id = track.id
            AND credit.artist_id = $1
      )
  )
ORDER BY state.updated_at, state.track_id
LIMIT $2
