SELECT track.sc_track_id AS "sc_track_id!",
       candidate.uploaded_generation AS "uploaded_generation!",
       candidate.updated_at AS "waiting_since!"
FROM (
    (
        SELECT walked.sc_track_id,
               walked.uploaded_generation,
               walked.updated_at,
               true AS fresh
        FROM (
            SELECT event.sc_track_id,
                   event.uploaded_generation,
                   event.updated_at
            FROM storage_event_state AS event
            WHERE event.uploaded_generation > 0
              AND event.transcription_generation IS NULL
              AND event.updated_at < now() - interval '10 minutes'
              AND (event.updated_at, event.sc_track_id)
                  > (coalesce($3, '-infinity'::timestamptz), coalesce($4, ''))
            ORDER BY event.updated_at, event.sc_track_id
            LIMIT $5
        ) AS walked
        CROSS JOIN LATERAL (
            SELECT 1
            FROM lyrics_cache AS lyrics
            WHERE lyrics.sc_track_id = walked.sc_track_id
              AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
              AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
              AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
            LIMIT 1
        ) AS plain
        ORDER BY walked.updated_at, walked.sc_track_id
    )
    UNION ALL
    (
        SELECT event.sc_track_id,
               event.uploaded_generation,
               event.updated_at,
               false
        FROM transcription_wire_state AS wire
        JOIN storage_event_state AS event
          ON event.sc_track_id = wire.sc_track_id
        JOIN lyrics_cache AS lyrics
          ON lyrics.sc_track_id = wire.sc_track_id
        WHERE wire.status = 'quarantined'
          AND wire.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
          AND wire.upload_generation < event.uploaded_generation
          AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
          AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
          AND lyrics.created_at < (now() AT TIME ZONE 'UTC') - interval '10 minutes'
          AND event.updated_at < now() - interval '10 minutes'
          AND event.uploaded_generation > 0
        ORDER BY event.updated_at, event.sc_track_id
    )
    ORDER BY updated_at, sc_track_id
) AS candidate
JOIN tracks AS track
  ON track.sc_track_id = candidate.sc_track_id
WHERE track.storage_state = 'ok'
  AND NOT track.needs_duration_resolve
  AND track.sc_track_id <> ALL ($2::text[])
  AND (
      (
          candidate.fresh
          AND track.transcribe_state IS NULL
          AND NOT EXISTS (
              SELECT 1
              FROM transcription_wire_state AS wire
              WHERE wire.sc_track_id = track.sc_track_id
          )
      )
      OR (
          NOT candidate.fresh
          AND track.transcribe_state = 'quarantined'
      )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM background_jobs AS job
      WHERE job.kind = 'lyrics.dispatch_transcription'
        AND job.dedup_key = track.sc_track_id
  )
ORDER BY candidate.updated_at, candidate.sc_track_id
FOR UPDATE OF track SKIP LOCKED
LIMIT $1
