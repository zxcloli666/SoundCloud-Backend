SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.star_due_marks (
    sc_track_id text NOT NULL,
    marked_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.star_due_watch (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    watched_at timestamptz NOT NULL
);

ALTER TABLE public.star_track_tiers
    ADD COLUMN IF NOT EXISTS recheck_at timestamptz DEFAULT now();

CREATE OR REPLACE FUNCTION public.star_due_mark()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM public.star_due_watch AS watch
        WHERE watch.watched_at > now() - interval '1 day'
    ) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        INSERT INTO public.star_due_marks (sc_track_id)
        SELECT star.sc_track_id
        FROM public.star_track_tiers AS star
        WHERE star.sc_track_id = NEW.sc_track_id;
        RETURN NULL;
    END IF;
    INSERT INTO public.star_due_marks (sc_track_id)
    SELECT star.sc_track_id
    FROM public.star_track_tiers AS star
    WHERE star.sc_track_id = OLD.sc_track_id;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id THEN
            INSERT INTO public.star_due_marks (sc_track_id)
            SELECT star.sc_track_id
            FROM public.star_track_tiers AS star
            WHERE star.sc_track_id = NEW.sc_track_id;
        END IF;
    END IF;
    RETURN NULL;
END
$$;
