SELECT event.uploaded_generation
FROM lyrics_cache AS lyrics
JOIN tracks AS track ON track.sc_track_id = lyrics.sc_track_id
JOIN storage_event_state AS event ON event.sc_track_id = track.sc_track_id
WHERE lyrics.sc_track_id = $1
  AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
  AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
  AND track.storage_state = 'ok'
  AND track.index_state <> 'too_long'
  AND track.storage_state <> 'too_long'
  AND NOT track.needs_duration_resolve
  AND track.transcribe_state IS NULL
  AND event.uploaded_generation > 0
  AND NOT EXISTS (
      SELECT 1
      FROM transcription_wire_state AS wire
      WHERE wire.sc_track_id = track.sc_track_id
  )
FOR UPDATE OF lyrics, track, event
