WITH by_artist AS (
    SELECT track.id
    FROM tracks AS track
    WHERE track.primary_artist_id = $1
    UNION
    SELECT credit.track_id
    FROM track_artists AS credit
    WHERE credit.artist_id = $1
)
SELECT state.sc_track_id
FROM by_artist
JOIN lyrics_lookup_state AS state ON state.track_id = by_artist.id
WHERE state.wake_message_id IS NOT NULL
  AND state.wake_durable_at IS NULL
ORDER BY state.updated_at, state.track_id
LIMIT $2
