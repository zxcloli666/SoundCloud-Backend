SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_work_keys (
    track_id uuid PRIMARY KEY,
    primary_artist_id uuid,
    superseded boolean NOT NULL DEFAULT false,
    work_key text,
    recording_key text,
    work_normalizer_version smallint
);

ALTER TABLE public.track_work_keys
    ADD CONSTRAINT track_work_keys_track_fk
    FOREIGN KEY (track_id) REFERENCES public.tracks(id) ON DELETE CASCADE NOT VALID;

CREATE INDEX IF NOT EXISTS track_work_keys_unkeyed_idx
    ON public.track_work_keys (track_id)
    WHERE work_normalizer_version IS NULL;

CREATE INDEX IF NOT EXISTS track_work_keys_work_idx
    ON public.track_work_keys (work_key, primary_artist_id)
    WHERE work_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS track_work_keys_duplicate_scan_idx
    ON public.track_work_keys (primary_artist_id, recording_key, track_id)
    WHERE recording_key IS NOT NULL
        AND primary_artist_id IS NOT NULL
        AND NOT superseded;
