ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS pipeline_held boolean NOT NULL DEFAULT false;

DROP TRIGGER IF EXISTS tracks_lyrics_lookup_state_refresh ON tracks;

CREATE TRIGGER tracks_lyrics_lookup_state_refresh
AFTER INSERT OR UPDATE OF
    sc_track_id,
    title,
    metadata_artist,
    uploader_username,
    duration_ms,
    genius_song_id,
    genius_url,
    release_date,
    sc_created_at,
    index_priority,
    pipeline_held
ON tracks
FOR EACH ROW
WHEN (NOT NEW.pipeline_held)
EXECUTE FUNCTION lyrics_lookup_track_state_refresh();
