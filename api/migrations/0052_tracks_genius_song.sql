ALTER TABLE tracks ADD COLUMN IF NOT EXISTS genius_song_id bigint;
ALTER TABLE tracks ADD COLUMN IF NOT EXISTS genius_url text;
