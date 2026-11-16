DO $$
DECLARE
    tries integer := 0;
BEGIN
    IF public.scd_role() <> 'core' OR public.scd_mode() <> 'split' OR NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_class
        WHERE relnamespace = 'public'::regnamespace
          AND relname IN (
              'artists_discover_az_idx',
              'artists_discover_listeners_idx',
              'artists_discover_popular_idx',
              'artists_discover_star_idx',
              'artists_discover_tracks_idx',
              'artists_discover_trending_idx',
              'artists_search_doc_gin',
              'artists_tags_gin'
          )
    ) THEN
        RETURN;
    END IF;
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    LOCK TABLE public.artists IN SHARE UPDATE EXCLUSIVE MODE;
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.artists IN ACCESS EXCLUSIVE MODE;
            EXIT;
        EXCEPTION WHEN lock_not_available THEN
            tries := tries + 1;
            IF tries = 120 THEN
                RAISE;
            END IF;
            PERFORM pg_catalog.pg_sleep(1);
        END;
    END LOOP;
    DROP INDEX IF EXISTS
        public.artists_discover_az_idx,
        public.artists_discover_listeners_idx,
        public.artists_discover_popular_idx,
        public.artists_discover_star_idx,
        public.artists_discover_tracks_idx,
        public.artists_discover_trending_idx,
        public.artists_search_doc_gin,
        public.artists_tags_gin;
END
$$;
