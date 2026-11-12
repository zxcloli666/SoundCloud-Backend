SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_search (
    id uuid PRIMARY KEY,
    sc_track_id text NOT NULL,
    uploader_sc_user_id text,
    title text NOT NULL,
    title_normalized text NOT NULL,
    metadata_artist text,
    uploader_username text,
    duration_ms integer NOT NULL,
    play_count_sc bigint,
    variant boolean NOT NULL
);
