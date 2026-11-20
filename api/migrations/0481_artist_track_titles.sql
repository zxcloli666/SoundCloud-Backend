SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.artist_track_titles (
    artist_id uuid NOT NULL,
    title_normalized text COLLATE "C" NOT NULL,
    track_id uuid NOT NULL,
    PRIMARY KEY (artist_id, title_normalized, track_id)
);

CREATE INDEX IF NOT EXISTS artist_track_titles_track_idx
    ON public.artist_track_titles (track_id);
