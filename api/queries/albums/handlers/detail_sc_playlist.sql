SELECT p.urn AS "urn!"
FROM albums al
JOIN artists ar ON ar.id = al.primary_artist_id
JOIN playlists p ON p.owner_sc_user_id = ar.sc_user_id
WHERE al.id = $1
  AND p.deleted_at IS NULL
  AND p.sharing = 'public'
  AND p.title_normalized = al.normalized_title
  AND p.playlist_type = al.type
ORDER BY (SELECT count(*)
          FROM album_tracks at
          JOIN tracks t ON t.id = at.track_id
          JOIN playlist_track_projection m ON m.sc_track_id = t.sc_track_id
          WHERE at.album_id = al.id
            AND m.playlist_urn = p.urn) DESC,
         p.track_count DESC NULLS LAST,
         p.urn
LIMIT 1
