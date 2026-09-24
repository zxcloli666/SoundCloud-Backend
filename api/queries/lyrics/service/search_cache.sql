SELECT track.sc_track_id,
       track.metadata_artist,
       track.uploader_username,
       track.duration_ms,
       cache.synced_lrc,
       cache.plain_text,
       cache.source,
       cache.language,
       cache.language_confidence
FROM tracks AS track
JOIN lyrics_cache AS cache ON cache.sc_track_id = track.sc_track_id
WHERE track.title_normalized = $1
  AND (
      NULLIF(btrim(cache.plain_text), '') IS NOT NULL
      OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL
  )
ORDER BY track.index_priority,
         track.play_count_sc DESC NULLS LAST,
         track.id
LIMIT 100
