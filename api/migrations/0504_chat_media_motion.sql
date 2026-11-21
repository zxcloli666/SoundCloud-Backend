SET LOCAL lock_timeout = '2s';

ALTER TABLE chat.media DROP CONSTRAINT IF EXISTS media_kind_check;
ALTER TABLE chat.media ADD CONSTRAINT media_kind_check CHECK (kind IN ('image', 'thumb', 'motion', 'avatar', 'backdrop'));
