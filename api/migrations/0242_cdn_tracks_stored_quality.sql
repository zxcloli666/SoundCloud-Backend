SET LOCAL lock_timeout = '5s';

ALTER TABLE cdn_tracks ADD COLUMN IF NOT EXISTS stored_quality varchar(8);
