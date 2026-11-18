SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.artist_playable_tracks (
    artist_id uuid NOT NULL,
    track_id uuid NOT NULL,
    sc_track_id text NOT NULL,
    ready_home boolean NOT NULL,
    ready_wave boolean NOT NULL,
    PRIMARY KEY (artist_id, track_id)
);

CREATE INDEX IF NOT EXISTS artist_playable_tracks_track_idx
    ON public.artist_playable_tracks (track_id);

CREATE INDEX IF NOT EXISTS artist_playable_tracks_read_idx
    ON public.artist_playable_tracks (artist_id) INCLUDE (sc_track_id, ready_home, ready_wave);
