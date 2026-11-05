SET LOCAL lock_timeout = '2s';

ALTER TABLE chat.media DROP CONSTRAINT IF EXISTS media_kind_check;
ALTER TABLE chat.media ADD CONSTRAINT media_kind_check CHECK (kind IN ('image', 'thumb', 'avatar', 'backdrop'));

ALTER TABLE chat.conversations ADD COLUMN IF NOT EXISTS backdrop_fence uuid;

ALTER TABLE chat.reports ADD COLUMN IF NOT EXISTS kind text NOT NULL DEFAULT 'messages';
ALTER TABLE chat.reports ADD COLUMN IF NOT EXISTS media_id uuid;
ALTER TABLE chat.reports ADD COLUMN IF NOT EXISTS room_public boolean NOT NULL DEFAULT false;
ALTER TABLE chat.reports DROP CONSTRAINT IF EXISTS reports_kind_check;
ALTER TABLE chat.reports ADD CONSTRAINT reports_kind_check CHECK (kind IN ('messages', 'backdrop') AND (kind = 'messages') = (media_id IS NULL));

CREATE INDEX IF NOT EXISTS reports_backdrop ON chat.reports (conversation_id, media_id, created_at) WHERE kind = 'backdrop';
