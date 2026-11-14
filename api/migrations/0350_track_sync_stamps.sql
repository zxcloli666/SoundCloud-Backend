SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_sync_stamps (
    track_id uuid PRIMARY KEY,
    sc_synced_at timestamptz NOT NULL
);

ALTER TABLE public.track_sync_stamps
    ADD CONSTRAINT track_sync_stamps_track_fk
    FOREIGN KEY (track_id) REFERENCES public.tracks(id) ON DELETE CASCADE NOT VALID;
