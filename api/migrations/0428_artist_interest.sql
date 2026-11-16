SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.artist_interest (
    artist_id uuid PRIMARY KEY,
    score real NOT NULL,
    CONSTRAINT artist_interest_score_positive CHECK (score > 0)
);

DO $$
BEGIN
    IF public.scd_role() = 'core' THEN
        LOCK TABLE public.artists IN SHARE ROW EXCLUSIVE MODE;
        INSERT INTO public.artist_interest (artist_id, score)
        SELECT id, interest_score
        FROM public.artists
        WHERE interest_score > 0;
    END IF;
END
$$;

ALTER TABLE public.artist_interest
    ADD CONSTRAINT artist_interest_artist_fk
    FOREIGN KEY (artist_id) REFERENCES public.artists(id) ON DELETE CASCADE NOT VALID;
