SELECT track.title AS "title!",
       COALESCE(artist.name, track.metadata_artist, track.uploader_username, '') AS "artist!",
       track.duration_ms AS "duration_ms!",
       track.artwork_url AS "artwork_url!",
       COALESCE(NULLIF(lyrics.synced_lrc, ''), NULLIF(lyrics.plain_text, '')) AS lyrics
FROM tracks AS track
LEFT JOIN artists AS artist ON artist.id = track.primary_artist_id
LEFT JOIN LATERAL (
    SELECT cache.synced_lrc, cache.plain_text
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id = track.sc_track_id
    ORDER BY cache.updated_at DESC NULLS LAST
    LIMIT 1
) AS lyrics ON true
WHERE track.sc_track_id = $1
  AND track.artwork_url IS NOT NULL
  AND track.duration_ms > 0
