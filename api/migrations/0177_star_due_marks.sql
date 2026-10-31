SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS star_due_marks (
    sc_track_id text NOT NULL,
    marked_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS star_due_watch (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    watched_at timestamptz NOT NULL
);

ALTER TABLE star_track_tiers
    ADD COLUMN IF NOT EXISTS recheck_at timestamptz DEFAULT now();

CREATE OR REPLACE FUNCTION public.star_due_mark_track(track text)
RETURNS void
LANGUAGE sql
AS $$
    INSERT INTO public.star_due_marks (sc_track_id)
    SELECT star.sc_track_id
    FROM public.star_track_tiers AS star
    WHERE star.sc_track_id = track
      AND EXISTS (
          SELECT 1
          FROM public.star_due_watch AS watch
          WHERE watch.watched_at > now() - interval '1 day'
      )
$$;

CREATE OR REPLACE FUNCTION public.star_due_mark()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        PERFORM public.star_due_mark_track(NEW.sc_track_id);
        RETURN NULL;
    END IF;
    PERFORM public.star_due_mark_track(OLD.sc_track_id);
    IF TG_OP = 'UPDATE' THEN
        IF NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id THEN
            PERFORM public.star_due_mark_track(NEW.sc_track_id);
        END IF;
    END IF;
    RETURN NULL;
END
$$;
