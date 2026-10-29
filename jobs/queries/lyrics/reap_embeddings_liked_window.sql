WITH window_end AS MATERIALIZED (
    SELECT lyrics.created_at
    FROM lyrics_cache AS lyrics
    WHERE lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IS NULL
      AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
    ORDER BY lyrics.created_at, lyrics.sc_track_id
    OFFSET $1
    LIMIT 1
)
SELECT lyrics.sc_track_id AS "sc_track_id!"
FROM lyrics_cache AS lyrics
WHERE lyrics.embedded_at IS NULL
  AND lyrics.embedding_state IS NULL
  AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
  AND lyrics.created_at <= coalesce((SELECT created_at FROM window_end), 'infinity')
  AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
