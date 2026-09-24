SELECT lyrics.sc_track_id AS "sc_track_id!"
FROM lyrics_cache AS lyrics
JOIN tracks AS track
  ON track.sc_track_id = lyrics.sc_track_id
LEFT JOIN lyrics_embedding_wire_state AS wire
  ON wire.sc_track_id = lyrics.sc_track_id
WHERE lyrics.embedded_at IS NULL
  AND lyrics.embedding_state IS NULL
  AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
  AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
  AND (wire.sc_track_id IS NULL OR wire.status <> 'pending')
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.embed'
        AND job.dedup_key = lyrics.sc_track_id
  )
ORDER BY lyrics.created_at, lyrics.sc_track_id
FOR UPDATE OF lyrics SKIP LOCKED
LIMIT $1
