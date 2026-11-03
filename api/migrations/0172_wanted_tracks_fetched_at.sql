SET LOCAL lock_timeout = '5s';

ALTER TABLE wanted_tracks ADD COLUMN IF NOT EXISTS fetched_at timestamptz;
