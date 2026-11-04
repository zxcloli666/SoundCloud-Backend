SELECT lyrics.sc_track_id AS "sc_track_id!"
FROM lyrics_cache AS lyrics
WHERE lyrics.embedded_at IS NULL
  AND lyrics.embedding_state IS NULL
  AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
  AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
ORDER BY lyrics.created_at
FETCH FIRST ($1::bigint + 1) ROWS WITH TIES
