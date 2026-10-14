WITH window_end AS MATERIALIZED (
    SELECT lyrics.created_at
    FROM lyrics_cache AS lyrics
    WHERE lyrics.embedded_at IS NULL
      AND lyrics.embedding_state IS NULL
      AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
    ORDER BY lyrics.created_at, lyrics.sc_track_id
    OFFSET $4
    LIMIT 1
)
SELECT lyrics.sc_track_id AS "sc_track_id!",
       lyrics.created_at AT TIME ZONE 'UTC' AS "waiting_since!"
FROM lyrics_cache AS lyrics
LEFT JOIN lyrics_embedding_wire_state AS wire
  ON wire.sc_track_id = lyrics.sc_track_id
WHERE lyrics.embedded_at IS NULL
  AND lyrics.embedding_state IS NULL
  AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
  AND lyrics.created_at <= coalesce((SELECT created_at FROM window_end), 'infinity')
  AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
  AND (wire.sc_track_id IS NULL OR wire.status <> 'pending')
  AND (
      SELECT track.index_priority
      FROM tracks AS track
      WHERE track.sc_track_id = lyrics.sc_track_id
  ) = $2
  AND lyrics.sc_track_id <> ALL ($3::text[])
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.embed'
        AND job.dedup_key = lyrics.sc_track_id
  )
ORDER BY lyrics.created_at, lyrics.sc_track_id
FOR UPDATE OF lyrics SKIP LOCKED
LIMIT $1
