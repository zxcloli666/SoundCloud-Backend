SET LOCAL lock_timeout = '5s';

ALTER TABLE public.lyrics_cache
    ADD COLUMN IF NOT EXISTS embedding_request_id varchar(128),
    ADD COLUMN IF NOT EXISTS embedding_request_created_at timestamp;
