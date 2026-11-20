SET LOCAL lock_timeout = '5s';

ALTER TABLE public.lyrics_cache ADD COLUMN IF NOT EXISTS synced_words text;
